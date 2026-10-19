
# Contact Manager (ContactMgr) 设计文档

- 2026-10-17：新增好友关系 `friend_state` 与好友请求、tag 三类与写权限、`contact.lookup`、`remove_contact` / `unblock_contact`、联系人变更事件（§3.4–§3.9）；`AccessGroupLevel::Friend` 改名 `Allow`，`Contact.groups` 删除。当前行为以代码为准。

## 1. 概述 (Overview)

ContactMgr 是 MessageCenter 消息系统的核心组件，负责管理 **身份关联（Identity Association）** 与 **关系（Relationship）**。它是“通讯录 + 准入策略”，连接外部通讯平台（Telegram, Email 等）的 endpoint 与内部联系人（DID），同时承担**访问控制（Access Control）和多端身份聚合**的职责。

术语与投递语义以两份主设计文档为准：[Message Center.md](<./Message Center.md>)、[Message Tunnel Design.md](<./Message Tunnel Design.md>)。好友消息与关系状态机见 [My Network相关协议设计.md](<./My Network相关协议设计.md>) §5–§7。

### 核心职责

1. **关联**：登记与维护 shadow endpoint DID（`did:msgtunnel:*`）与正式联系人的 binding。
2. **展示**：为 UI 提供联系人名片、头像、endpoint 列表与 reverse lookup（由 endpoint 反查联系人）。
3. **生命周期管理**：管理联系人从“陌生人（Shadow）”到“熟人（Verified）”的转化。
4. **身份聚合**：处理多渠道账号的合并（Merge），解决数据冲突。
5. **动态访问控制（ACL）**：基于准入级别或上下文（临时会话）决定入站消息进入 `INBOX / REQUEST_BOX / DROP`。
6. **好友关系**：维护 `friend_state`，代 owner 收发好友请求（§3.4）。
7. **分组**：tag 与按类别的写权限（§3.5）。
8. **查找**：在 owner 的关系、本 Zone 与名字服务中找人和群（§3.6）。

启动或重载设置时，Zone 用户联系人同步到系统作用域、每个有效 Zone 用户的实际 DID 作用域，以及 tunnel 配置中的 owner 作用域。同 Zone 用户默认是好友（`friend_state = friend`，准入 `Allow`，进入 `INBOX`），已有 `Block` 设置保持不变。同步不会迁移历史 `REQUEST_BOX` 记录。

实现：`frame/msg_center/src/contact_mgr.rs`（存储、准入、迁移）、`contact_rpc.rs`（§3.4–§3.7 的 RPC 与授权）、`friend_service.rs`（好友请求）、`contact_lookup.rs`（查找）、`network_directory.rs`（DID → Zone）。共享类型在 `buckyos-api` 的 `msg_center_client.rs` 与 `msg_contact.rs`。

### 非职责（冻结）

- **ContactMgr 不参与出站选路。** 发送目标由调用方在构造 `MsgObject` 之前显式确定（用户点选或 Agent 沿用会话中的 endpoint DID）；`post_send` 不查询 ContactMgr。**ContactMgr 的关联结果不得自动成为发送目的地。**
- ContactMgr 的关联**不改写消息来源**：入站消息的 `from` 保持 shadow endpoint DID，关联只影响展示与 ACL。
- 已废弃的发送模型（不得再出现）：`contact DID + selector`、发送路径上的 `resolve_target(contact, "telegram")`、`get_preferred_binding()` 参与投递、按 `last_active_at` 选信道、`preferred_tunnel` / default tunnel。

---

## 2. 核心概念与数据模型 (Data Models)

### 2.1 基础定义

* **DID (Decentralized Identifier)**: 系统内部通用的唯一用户标识。
* **Shadow Contact (影子联系人)**: 系统基于收到消息自动创建的临时联系人，未经用户确认，置信度低。
* **Verified Contact (正式联系人)**: 用户手动导入、创建或确认过的联系人，置信度高。

### 2.2 数据结构定义 (Rust 风格伪代码)

```rust
// 1. 账号绑定：描述外部平台的身份
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountBinding {
    pub platform: String,       // e.g., "telegram", "email", "wechat"
    pub account_id: String,     // 平台唯一ID, e.g., "123456789" (TG UserID)
    pub display_id: String,     // 可读ID, e.g., "@username"
    pub tunnel_instance_id: String, // 稳定短实例 id (如 "tg-main-tunnel")，非 transport_did
    pub account_type: String,   // 平台实体类型: user/group/channel/addr (非 endpoint 为空)
    pub endpoint_did: Option<DID>, // shadow endpoint DID: did:msgtunnel:<enc>.<type>.<tunnel_instance_id>
    pub last_active_at: u64,    // 最后活跃时间。仅限 UI 展示与 merge 冲突裁决使用，不参与投递选路
    pub meta: Map<String, String>, // 平台特定的额外数据 (头像URL等)
}

// 2. 联系人来源：决定置信度和合并策略
pub enum ContactSource {
    ManualImport,   // 用户导入 (高优先级)
    ManualCreate,   // 用户新建 (高优先级)
    AutoInferred,   // 收到消息自动建立 (低优先级，即 "Shadow Contact")
    Shared,         // 他人分享 (中优先级)
}

// 3. 访问权限等级：只是投递策略，好友关系看 friend_state
pub enum AccessGroupLevel {
    Block,          // 黑名单：直接丢弃消息
    Stranger,       // 陌生人：放入 Request Box，静默
    Temporary,      // 临时授权：放入 Inbox (有过期时间)
    Allow,          // 允许：放入 Inbox（原名 Friend，线上值 "allow"）
}

// 4. 临时授权凭证 (用于 Temporary 级别)
pub struct TemporaryGrant {
    pub context_id: String,     // 授权上下文, e.g., "reply_thread_101"
    pub granted_at: u64,
    pub expires_at: u64,        // 过期时间戳
}

// 5. 好友关系（owner 一侧所知，buckyos_api::msg_contact）
pub enum FriendState {
    None,             // 默认
    OutgoingPending,  // 我发出的请求待对方确认
    IncomingPending,  // 对方的请求待我处理
    Friend,
}

// 6. 联系人聚合实体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    pub did: DID,               // 主键

    // === 基础信息 ===
    pub name: String,           // 显示名称
    pub avatar: Option<String>,
    pub note: Option<String>,   // 备注

    // === 状态与来源 ===
    pub source: ContactSource,
    pub is_verified: bool,      // source != AutoInferred

    // === 身份绑定 ===
    pub bindings: Vec<AccountBinding>, // 一个 DID 对应多个外部账号

    // === 权限控制 ===
    pub access_level: AccessGroupLevel,
    pub temp_grants: Vec<TemporaryGrant>, // 仅当 level=Temporary 时有效

    // === 关系 ===
    pub friend_state: FriendState, // 缺省为 None

    // === 分组 ===
    pub tags: Vec<String>,      // 系统 / 应用 / 用户 tag，见 §3.5

    pub created_at: u64,
    pub updated_at: u64,
}

```

- `ContactPatch`（`update_contact`）可改 `name / avatar / note / access_level / source / is_verified`。tags 只经 `update_tags` 改（§3.5），`friend_state` 只由好友流程、Zone 同步、合并与迁移改。
- `ContactQuery`（`list_contacts`）可按 `source`、`access_level`、`friend_state`、`tag`（精确匹配）过滤；`keyword` 对 did、name、note、tags 与 bindings 做不区分大小写的子串匹配。
- `ImportContactEntry` 带 `name / avatar / note / bindings / tags`。

---

## 3. 核心机制设计

### 3.1 身份解析与自动创建 (Resolution & Shadowing)

当 Tunnel 收到消息时：

1. **Endpoint DID 确定性构造**：`resolve_endpoint_did(platform, account_id, account_type, tunnel_instance_id)` 得到 `did:msgtunnel:*`。这一步是纯函数，不依赖任何查询，消息的 `from` 就是它，且永不被后续关联改写。
2. **Lookup**: 查询该 endpoint 是否已关联某个 Contact。
3. **Hit**: 命中则用该 Contact 做 ACL 判断与 UI 展示。
4. **Miss (Auto-Create)**: 未命中则创建 Shadow Contact：
* 创建 `Contact`，标记 `source = AutoInferred`，`access_level = Stranger`。
* 登记 `AccountBinding`（含 `endpoint_did`）。
* 该 Contact 只影响展示与 ACL；**不影响消息的 `from/to`，也不产生任何投递路径**。



5. **原生 DID 发送者不自动创建**：发来普通消息的未知原生 DID（例如另一 Zone 用户，或尚未同步进来的新 Zone 成员）只按 Stranger 进入 REQUEST_BOX，不写联系人记录。
   以下操作才创建联系人（准入均为 `Stranger`，除非操作本身改准入）：`block_contact`、`grant_temporary_access`、携带 `access_level` 的 `update_contact`（`source = ManualCreate`，可带 `name`）、`send_friend_request`（`ManualCreate`）、收到好友请求（`AutoInferred`，用对方的 `profile_hint` 补名称与头像）、`update_tags`（用户调用为 `ManualCreate`，服务调用为 `AutoInferred`）。
   不带 `access_level` 的其它字段修改仍要求联系人已存在。“仅保存为联系人”即 `update_contact{access_level: stranger}`。

**Zone 用户同步**：启动、`reload_settings` 以及每 30 秒一次的周期扫描读取 Zone 用户列表与资料，在列表变化时写入系统库和各 owner 库（`source = Shared`，tag `zone_user`）。
每次同步都把 `friend_state` 置为 `friend`；DID 首次成为 Zone 用户联系人（还没有 `zone_user` tag）时授予 `Allow`（Block 保留），并写入用户资料中配置的 tags（`UserContactSettings.tags`）。之后的同步只更新名称、头像、备注、绑定，tag 只保证 `zone_user` 还在，不覆盖 owner 手动设置的准入级别，owner 删掉的其他 tag 也不会被加回。只有内容实际变化的联系人才写库并发变更事件（§3.8）。名称与头像取用户 profile 的公开投影（`to_public_profile()`）：头像以 Zone 公布的值为准，公开值消失（清除或改为私有）时同步撤下联系人上的旧头像；备注、屏蔽状态不受影响。同步指纹包含头像摘要，只换头像也会触发同步；control-panel 保存用户或 Agent 资料后会调用 `reload_settings` 立即同步。
control-panel 创建 / 修改用户不通知 msg-center，新成员最迟一个扫描周期后成为彼此的好友。用户资料中的 `UserContactSettings.groups` 不再被读取。

同一轮同步读取 `users/{user}/agents/{id}/spec` 中的 AgentDocument，以及 `agents/{id}/doc`，过滤已删除的 Agent，再按 `AgentDocument.owner` 为 Agent（包括 Jarvis）与主人建立双向好友，无需配置 tunnel。主人库中的 Agent 只带 tag `zone_agent`（不再写 `agent`），Agent 库中的主人带 tag `zone_user`；`friend_state` 每次同步置为 `friend`，只在首次同步该关系时授予 `Allow`，保留 Block 和之后手动调整的准入级别。用户身份默认采用 profile 中的实际 DID。Agent 列表、owner、显示名称或头像（`AgentProfile.avatar`）变化也会触发同步。

**授权**：`contact.*` 网络调用按 `contact_mgr_owner` 授权。指定 owner 时，写操作要求调用者就是该 owner（服务主体沿用 RBAC `obj://msg-center/owners/*`），读操作另允许观察 Zone Agent；
未指定 owner 的系统库对所有已认证调用者可读，仅服务主体可写。无 token 的进程内调用视为服务内部调用。

§3.4–§3.7 的方法（`buckyos_api::CONTACT_EXT_METHODS`）省略 `contact_mgr_owner` 时作用于调用用户本人的库（旧方法省略时仍是系统库），没有用户身份返回 `contact_mgr_owner-required`：

| 方法 | 要求 |
| --- | --- |
| `contact.lookup`、`contact.list_friend_requests` | 对 owner 的读权限 |
| `contact.remove_contact`、`contact.unblock_contact` | 对 owner 的写权限 |
| `contact.send_friend_request`、`respond_friend_request`、`cancel_friend_request`、`remove_friend` | 对 owner 的写权限，且 owner 由本 Zone 托管（否则 `owner-not-hosted`） |
| `contact.update_tags` | 按 tag 类别，见 §3.5 |

### 3.2 动态访问控制 (ACL & Temporary Access)

`MsgCenter` 在分发消息前（Dispatch），必须通过 ContactMgr 检查权限。

**检查逻辑 `check_access(did)`：**

1. **Block 检查**: 若 `level == Block`  **Reject**。
2. **Allow 检查**: 若 `level == Allow`  **Allow**。
3. **Temporary 检查**: 若 `level == Temporary`：
* 遍历 `temp_grants`，清除已过期的 Grant。
* 如果有任意一个 Grant 仍有效  **Allow**。
* 如果所有 Grant 均过期  **降级为 Stranger**  **Silence**。


4. **Stranger 检查**: 默认  **Silence/RequestBox**。

准入不看 `friend_state`。成为好友时把准入设为 `Allow`、解除好友时把 `Allow` 降为 `Stranger`（§3.4），用户之后单独调整准入不影响好友关系。`buckyos.friend` 消息在准入之前由好友流程处理（§3.4）。

### 3.3 合并策略 (Merge Strategy)

处理“影子联系人”与“导入联系人”的冲突。

* **策略 A：导入命中影子 (Import hits Shadow)**
* 场景：用户导入通讯录，其中一个号码对应已经发过消息的陌生人。
* 动作：**原地升级**。
* 结果：保留 DID，将 `source` 改为 `ManualImport`，`name` 更新为导入名称，保留历史聊天记录。


* **策略 B：影子命中导入 (Shadow hits Import)**
* 场景：用户先导入了联系人，后来这个人第一次发消息过来。
* 动作：**自动关联**。
* 结果：Tunnel 解析时直接返回已存在的 DID。


* **策略 C：显式合并 (Manual Merge)**
* 场景：系统检测到两个不同的 DID (UserA-TG, UserA-Email) 可能是一人。
* 动作：将 `SourceDID` 的所有 Bindings、Tags、Grants 移动到 `TargetDID`，准入按 Block > Allow > Temporary > Stranger 取靠前者，`friend_state` 取较强者（friend > incoming_pending > outgoing_pending > none），然后标记 `SourceDID` 为 `Merged/Deleted`。好友请求记录不随之迁移，仍挂在源 DID 下（已知缺口）。

### 3.4 好友关系与好友请求

`friend_state` 是 owner 一侧所知的好友关系，只由以下几处改变：好友流程（`friend_service.rs`）、Zone 同步（§3.1）、合并（§3.3）与存量迁移（§3.9）。

- **成为好友**：`friend_state = friend`；`access_level` 不是 Block 时设为 `Allow` 并清空临时授权。
- **结束好友**：`friend → none`，`Allow` 降为 `Stranger`，其他级别不变。
- **请求结束**（拒绝、忽略、取消、过期）只把对应的 pending 状态改回 `none`。
- 带系统 tag 的联系人是系统关系：不能解除好友，也不能删除（`system-relation`）。

请求记录存于 msg-center 主库表 `friend_requests`（主键 `(owner_key, peer_did, request_id)`，DDL 见 Message Center Durable Data Schema.md），两侧各存一份：

```text
outgoing: pending --收到 accept / 交叉请求--> accepted
          pending --收到 reject--> rejected
          pending --cancel_friend_request / remove_contact / block_contact--> cancelled
          pending --到期--> expired
incoming: pending --accept（或我对对方 send_friend_request）--> accepted
          pending --reject--> rejected
          pending --ignore / remove_contact / block_contact--> ignored
          pending --收到 cancel--> cancelled
          pending --到期--> expired
```

- 有效期 7 天（`FRIEND_REQUEST_TTL_MS`）。msg-center 每 10 分钟清扫一次；`list_friend_requests`、`respond`、`cancel` 也会即时处理过期请求。
- 冷却 24 小时（`FRIEND_REQUEST_COOLDOWN_MS`）：接收方拒绝或忽略后丢弃同一对方的新请求；发送方收到 `reject` 后拒绝再发（`friend-request-cooldown`）。
- 附言 ≤ 500 字符（`FRIEND_REQUEST_MESSAGE_MAX_CHARS`）。
- 好友状态变化在 msg-center 进程内串行，先落库再发消息；每次保存请求记录都发一条 `operation = friend_request` 的变更事件（§3.8）。
- 出向请求的 `delivery`：本 Zone 对方立即得到 `delivered` / `failed`；他 Zone 对方按原生 DeliveryRecord 换算。

五个 RPC、`buckyos.friend` 的消息格式、收到各动作的处理与发送前的判断见 My Network 相关协议设计 §5–§6。

### 3.5 分组 tag

tag 按名字分三类（`contact_tag_kind`）：

| 类别 | 判定 | 谁能写 |
| --- | --- | --- |
| 系统 | `zone_user`、`zone_agent` | 只有 msg-center 的 Zone 同步 |
| 应用 | `<app>.<name>`：`app` 以小写字母开头、只含 `[a-z0-9_-]`，`name` 只含 `[a-z0-9_.-]` | 服务 `system:<app>` |
| 用户 | 其他 | 用户本人 |

tag 存储前去掉首尾空白，须非空、≤ 64 字符、不含换行，否则 `invalid-tag`。

well-known tag（`WELL_KNOWN_CONTACT_TAGS`，buckyos-api 与 websdk 各一份；显示名由 Desktop i18n 提供）：

| key | 类别 | 所属应用 | 用户可编辑成员 |
| --- | --- | --- | --- |
| `zone_user` | 系统 | — | 否 |
| `zone_agent` | 系统 | — | 否 |
| `homestation.following` | 应用 | homestation | 否 |

写入：`contact.update_tags { did, add?, remove?, contact_mgr_owner? }` → 更新后的 Contact；联系人不存在时创建（准入 `Stranger`）。

- **用户 principal**：要求对 owner 的写权限（只能改自己的库）；`add` / `remove` 只能有用户 tag，否则 `tag-not-writable`。
- **其他 principal**：token 的 RBAC 主体须是 `system:<svc>`；要求对 owner 的读权限；tag 只能是 `<svc>.*`；另需 RBAC `obj://msg-center/contact_tags/<svc>`（`CONTACT_TAGS_RBAC_RESOURCE_PREFIX`）的 `write`。当前只有 `p, system:homestation, obj://msg-center/contact_tags/homestation,write,allow`（`rbac_config.rs`）。
- 无 token 的进程内调用不检查。

`update_contact` 不能改 tags。`import_contacts` 只保留条目中的用户 tag，系统 tag 与应用 tag 被丢弃。Zone 同步写系统 tag，并在联系人首次成为 Zone 联系人时写入 Zone 用户资料中配置的 tags（§3.1），这些 tag 不按类别检查。

### 3.6 查找（`contact.lookup`）

`contact.lookup { input, kinds?, limit?（默认 20，≤ 100）, contact_mgr_owner? }` → `LookupResult { input, input_kind, candidates, error? }`。没有网络目录：候选只来自 owner 自己的关系、本 Zone，以及名字服务对输入本身的解析。

| 输入 | `input_kind` | 处理 |
| --- | --- | --- |
| 空 | `empty` | 列出联系人与已加入的群 |
| `<group_did>?invite=<token>`，或 query 里同时有 `group` / `group_did` 与 `invite` 的 URL | `invite_link` | 解析该群 |
| `did:` 开头 | `did` | 解析该 DID |
| HTTP(S) 站点根地址（可带末尾 `/`） | `name` | 解析域名对应的 DID；已配置的 Web3 bridge 域名还原为对应方法的 DID，例如 `https://lurenpluto.web3.buckyos.ai` → `did:bns:lurenpluto` |
| 只含小写字母、数字、`-`、`.`，且 `DID::from_friendly_name` 能解析（BNS 名、域名） | `name` | 关键字匹配；本地已有联系人的显示名等于输入，或其 DID 主体等于输入或以 `<输入>.` 开头时不再解析，否则解析 |
| 其他 | `text` | 关键字匹配 |

不带协议的 Web3 bridge 域名也按配置还原 DID；普通友好名称沿用 `DID::from_friendly_name` 的规则。站点 URL 中的普通域名按 `did:web` 解析。

带 `?` 但不是邀请链接、DID 不合法、站点 URL 带凭据、非默认端口、非根路径或 fragment 时 `error.code = invalid-input`。

候选按段（`section`）排序，同一 DID 只留一条：

1. `resolved`：输入本身解析出的对象。依次查本 Zone 托管的群 → 已有联系人 → 本 Zone 的 DID（是否 Agent 由 token verifier 判断）→ `NetworkDirectory`。
2. `relation`：关键字匹配到的联系人（不含 owner 本人），以及 owner 的群：本 Zone 托管、owner 为 active / invited / pending_admin_approval 的群（按 DID 或名称匹配），和 owner 已同步的他 Zone 群（`joined_groups` 表，来自 `cyfs_dispatch.joined_groups` 绑定）。
3. `zone`：带系统 tag 的联系人，即本 Zone 用户与 owner 的 Agent。

每个候选含 `did`、`kind`（user / agent / group / zone / device / msgtunnel / unknown）、`name` / `avatar` / `note`、`reachability`、`relation`（`is_contact`、`friend_state`、`access_level`、有效临时授权的最晚到期时间、`tags`、任一方向的 pending `request_id`；群候选另有本人的 `group_state` / `group_role`）。群候选另有 `group`：`join_policy`、链接中的 `invite`、`hosted_here`、`host`、`owner`，以及成员数（仅当 `member_list_visibility = public`，或为 `members` 且本人是 active 成员，或本人是群主）。

`reachability.state`：`local`（本 Zone）、`tunnel`（`did:msgtunnel`）、`routable`（他 Zone，附 `zone`）、`unreachable`（附 `reason`：`no-zone` / `not-found` / `resolve-failed`）、`unknown`（关系段与 Zone 段候选不做网络检查）。

`error.code`：`invalid-input`；`not-found`（名字服务确认没有）；`unreachable`（名字服务暂时无回答）。按名称输入且本地有匹配时两者都不报。`forbidden` 已在类型中定义，lookup 目前不返回，权限不足是 RPC 错误。

`NetworkDirectory`（`network_directory.rs`）：用 name_client 解析 DID Document（先按 Owner 文档，失败再不指定类型；Owner 查询超时则不再追问）；找到与确认不存在的答案缓存 10 分钟，暂时无回答缓存 30 秒，单次 3 秒超时。name_client 拒绝解析的 DID（如 `did:dev` 这类密钥型 DID）按不存在处理，原生投递对它们直接 `native-route-unresolved`、不重试。DID 的 Zone：用户取 `binded_zone_list` 第一项，Zone 取自身，群取 `host`，其他取 `zone_did` / `zone`。原生投递的自动路由与 Zone 托管签名的校验也用它（Message Center.md §4.5）。

### 3.7 删除与解除屏蔽

- `contact.remove_contact { did }`：删除联系人记录、它的别名，以及群订阅中对它的引用；不删历史消息与请求记录。系统关系返回 `system-relation`，不存在返回 `not-found`。owner 由本 Zone 托管时先结清关系：结束好友并通知对方、撤回自己的 pending 请求、忽略对方的。
- `contact.unblock_contact { did }`：Block → 好友回到 `Allow`，否则 `Stranger`；不是 Block 时不变；不存在返回 `not-found`。
- `contact.block_contact` 与 `contact.update_contact{access_level: block}`：带 `contact_mgr_owner` 且 owner 由本 Zone 托管时，先撤回自己的 pending 请求（发 `cancel`）、忽略对方的；不结束好友关系，不通知对方。对方 DID 先按别名解析为 canonical DID。

### 3.8 变更事件

每次实际改变 owner 库的写入都发 kevent `/msg_center/<owner>/CONTACTS/changed`，`<owner>` 为 `owner.to_raw_host_name()`（如 `alice.bns.did`，见 `contact_changed_event_id`）。载荷 `ContactChangedEvent { owner, operation: upsert | remove | friend_request, dids, request_id?, updated_at_ms }`。

- `upsert`：`update_contact`、block / unblock、临时授权、导入、`update_tags`、好友状态变化、Zone 同步（只在内容变化时）；
- `remove`：删除联系人、合并的源 DID（目标 DID 另发 `upsert`）；
- `friend_request`：每次保存请求记录，带 `request_id`。

系统库没有事件。事件只是加速信号，订阅方收到后重新读取；HomeStation 订阅 `/msg_center/*/CONTACTS/**`。

### 3.9 数据迁移

加载 owner 库时改写旧 payload，随该库下一次保存写回：

- `groups` 并入 `tags`。带 `zone_user` / `zone_agent` 的联系人只保留 groups 里的系统 tag（这些 groups 是 Zone 的用户组，不是 owner 的分组）。
- `access_level: friend` → `allow`；其中带系统 tag、且还没有 `friend_state` 的联系人置为 `friend`（迁移前只有这类好友关系）。
- 还带 `groups` 的旧 payload 中，带 `zone_agent` 的联系人同时去掉旧版写入的 `agent` tag。
- 其他联系人 `friend_state = none`。

---

## 4. 接口定义 (API Interface)

```python
class ContactMgr:
    
    # ==========================
    # 1. 身份关联 (Tunnel 入站 / 系统调用)
    # ==========================
    def resolve_endpoint_did(self, platform, account_id, account_type, tunnel_instance_id) -> DID:
        """
        确定性构造 shadow endpoint DID：
        did:msgtunnel:<encoded_account_id>.<account_type>.<tunnel_instance_id>。
        同一四元组始终得到同一 DID。这是外部平台实体进入 from/to 的唯一入口。
        入参不全（缺 account_type 或 tunnel_instance_id）直接报错——
        历史的 did:bns:mc-* shadow fallback 已删除，不再有降级路径。
        """
        pass

    def resolve_contact_for_endpoint(self, endpoint_did: DID) -> Optional[DID]:
        """endpoint DID 反查归属的 canonical/contact DID（展示/审计用）。"""
        pass

    def resolve_canonical_did(self, did: DID) -> DID:
        """把 merge 后的 alias/tombstone DID 解析为当前 canonical DID。"""
        pass

    def list_alias_dids(self, canonical_did: DID) -> List[DID]:
        """列出 merge 到该 canonical DID 的所有 alias 源 DID。"""
        pass

    # ==========================
    # 1.1 UI 辅助（不属于发送路径）
    # ==========================
    def list_endpoints(self, contact_did: DID) -> List[AccountBinding]:
        """
        列出联系人的全部 endpoint，供 UI 展示为"可发起会话的信道列表"。
        用户点选其中一个 endpoint_did 作为 msg.to —— 这是构造 MsgObject
        之前的显式选择。ContactMgr 不提供"首选/默认"信道语义
        （get_preferred_binding 已删除），也不做任何自动挑选。
        resolve_target(contact, selector) 同样已从核心发送 API 移除，
        如保留仅作为联系人查看辅助（等价于 list_endpoints + 过滤）。
        """
        pass

    def lookup(self, input: str, kinds=None, limit=20) -> LookupResult:
        """在 owner 的关系、本 Zone 与名字服务中找人和群（§3.6）。"""
        pass

    # ==========================
    # 2. 权限与访问控制 (MsgCenter 调用)
    # ==========================
    def check_access_permission(self, did: DID) -> AccessLevel:
        """
        决定消息是否进入 Inbox。
        内部会自动处理 Temporary Grant 的过期清理和降级逻辑。
        """
        pass

    def grant_temporary_access(self, dids: List[DID], context: str, duration: int):
        """
        批量授权。
        场景：回复评论后，允许该评论区的人在 duration 秒内发消息。
        如果 DID 不存在，会先创建 Shadow Contact。
        """
        pass

    def block_contact(self, did: DID):
        """屏蔽；撤回 / 忽略双方 pending 好友请求，不结束好友关系。
        update_contact 把准入设为 Block 时同样处理。"""
        pass

    def unblock_contact(self, did: DID) -> Contact:
        """解除屏蔽：好友回到 Allow，否则 Stranger。"""
        pass
    
    # ==========================
    # 3. 联系人管理 (User/Agent 调用)
    # ==========================
    def list_contacts(self, query: ContactQuery) -> List[Contact]:
        """按 source / access_level / friend_state / tag 过滤，keyword 子串匹配。"""
        pass

    def import_contacts(self, contacts: List[ImportSchema], upgrade_to_friend=False) -> ImportReport:
        """
        批量导入。自动执行 "策略 A" (升级 Shadow Contact)。
        条目只保留用户 tag；upgrade_to_friend 只把准入设为 Allow，不改 friend_state。
        """
        pass

    def merge_contacts(self, target_did: DID, source_did: DID):
        """
        手动合并两个 DID。source 的 binding/grant/tag 迁移到 target，
        friend_state 取较强者；source DID 不删除，而是登记为 target 的
        alias/tombstone，使历史 MsgObject 和旧 session peer DID 仍可通过
        resolve_canonical_did 解释。
        """
        pass

    def update_contact(self, did: DID, **kwargs):
        """
        修改名称、头像、备注、准入级别等。带 access_level 时联系人不存在即创建。
        tags 用 update_tags。
        """
        pass

    def update_tags(self, did: DID, add: List[str], remove: List[str]) -> Contact:
        """增删 tag，调用者只能写自己那一类 tag（§3.5）。"""
        pass

    def remove_contact(self, did: DID):
        """删除联系人记录与别名，先结清好友关系；系统关系拒绝（§3.7）。"""
        pass

    # ==========================
    # 4. 好友请求（§3.4，协议见 My Network 相关协议设计 §5）
    # ==========================
    def send_friend_request(self, did: DID, message: Optional[str]) -> FriendRequestResult: pass
    def respond_friend_request(self, request_id: str, decision: "accept|reject|ignore") -> FriendRequestResult: pass
    def cancel_friend_request(self, request_id: str) -> FriendRequestResult: pass
    def remove_friend(self, did: DID, notify=True) -> FriendRequestResult: pass
    def list_friend_requests(self, direction=None, state=None, limit=50, offset=0) -> List[FriendRequest]: pass

```

---

## 5. 关键业务流程 (Scenarios)

### 场景一：回复评论触发临时会话

1. **用户操作**: 用户在 Agent 界面回复了一个帖子 (Thread ID: `T_100`)。
2. **Agent**: 提取帖子参与者列表 `[DID_A, DID_B]`。
3. **Agent -> ContactMgr**: 调用 `grant_temporary_access([DID_A, DID_B], "T_100", 24小时)`。
4. **ContactMgr**:
* 检查 `DID_A` 是否存在，若无则创建 Shadow Contact。
* 添加 `TemporaryGrant(context="T_100", expire=Now+24h)`。
* 设置 `access_level = Temporary`。


5. **结果**: 接下来的 24 小时内，`DID_A` 发来的消息会直接进入 Inbox 并提醒用户。

### 场景二：导入通讯录自动“洗白”陌生人

1. **现状**: `DID_X` 是一个 Shadow Contact (Telegram用户 @water)，之前发过消息，但在 Request Box 里。
2. **用户操作**: 导入手机通讯录，包含条目 `name="刘志聪", telegram="@water"`。
3. **ContactMgr -> import_contacts**:
* 解析导入条目，发现 Telegram handle `@water` 对应的 ID 匹配 `DID_X`。
* **执行合并**:
* `DID_X.name` 更新为 "刘志聪"。
* `DID_X.source` 更新为 `ManualImport`。
* `DID_X.access_level` 升级为 `Allow` (假设导入即信任；`friend_state` 不变)。




4. **结果**: 之前的对话记录保留，联系人变为“熟人”，后续消息正常提醒。

### 场景三：加好友

1. Alice 调用 `contact.send_friend_request { did: bob }`：Alice 库中 Bob 置 `outgoing_pending`，msg-center 发出 `buckyos.friend` `request`。
2. Bob 的 msg-center 建 incoming 请求记录，Bob 库中 Alice 置 `incoming_pending`，请求作为陌生人消息进 Bob 的 REQUEST_BOX。
3. Bob 调用 `contact.respond_friend_request { decision: accept }`：Bob 一侧成为好友（准入 `Allow`），回 `accept`；Alice 收到后同样成为好友。

---

## 6. UI/UX 建议

1. **影子标记**: 在消息列表中，Shadow Contact 的头像应有视觉区分（如灰色边框、虚线或问号角标），提示用户“未保存”。
2. **临时倒计时**: 对于 `Temporary` 权限的会话，标题栏显示倒计时（e.g., "临时会话：剩余 3 小时"）。
3. **来源展示**: 联系人详情页应展示来源（"来自 Telegram 自动添加" 或 "导入自 Google Contacts"）。
4. **好友显示**: 只有 `friend_state = friend` 显示为好友；`Allow` 只表示“允许消息”。
