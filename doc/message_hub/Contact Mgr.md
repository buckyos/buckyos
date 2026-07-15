
# Contact Manager (ContactMgr) 设计文档

## 1. 概述 (Overview)

ContactMgr 是 MessageCenter 消息系统的核心组件，负责管理 **身份关联（Identity Association）** 与 **关系（Relationship）**。它是“通讯录 + 准入策略”，连接外部通讯平台（Telegram, Email 等）的 endpoint 与内部联系人（DID），同时承担**访问控制（Access Control）和多端身份聚合**的职责。

术语与投递语义以两份主设计文档为准：[Message Center.md](<./Message Center.md>)、[Message Tunnel Design.md](<./Message Tunnel Design.md>)。

### 核心职责

1. **关联**：登记与维护 shadow endpoint DID（`did:msgtunnel:*`）与正式联系人的 binding。
2. **展示**：为 UI 提供联系人名片、头像、endpoint 列表与 reverse lookup（由 endpoint 反查联系人）。
3. **生命周期管理**：管理联系人从“陌生人（Shadow）”到“熟人（Verified）”的转化。
4. **身份聚合**：处理多渠道账号的合并（Merge），解决数据冲突。
5. **动态访问控制（ACL）**：基于关系（好友）或上下文（临时会话）决定入站消息进入 `INBOX / REQUEST_BOX / DROP`。

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

// 3. 访问权限等级
pub enum AccessGroupLevel {
    Block,          // 黑名单：直接丢弃消息
    Stranger,       // 陌生人：放入 Request Box / Spam，静默
    Temporary,      // 临时授权：放入 Inbox，允许通知 (有过期时间)
    Friend,         // 好友/白名单：放入 Inbox，允许通知
}

// 4. 临时授权凭证 (用于 Temporary 级别)
pub struct TemporaryGrant {
    pub context_id: String,     // 授权上下文, e.g., "reply_thread_101"
    pub granted_at: u64,
    pub expires_at: u64,        // 过期时间戳
}

// 5. 联系人聚合实体
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
    
    // === 分组与标签 ===
    pub groups: Vec<String>,    // e.g., "colleagues", "family"
    pub tags: Vec<String>,      // e.g., "lead", "developer"
    
    pub created_at: u64,
    pub updated_at: u64,
}

```

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



### 3.2 动态访问控制 (ACL & Temporary Access)

`MsgCenter` 在分发消息前（Dispatch），必须通过 ContactMgr 检查权限。

**检查逻辑 `check_access(did)`：**

1. **Block 检查**: 若 `level == Block`  **Reject**。
2. **Friend 检查**: 若 `level == Friend`  **Allow**。
3. **Temporary 检查**: 若 `level == Temporary`：
* 遍历 `temp_grants`，清除已过期的 Grant。
* 如果有任意一个 Grant 仍有效  **Allow**。
* 如果所有 Grant 均过期  **降级为 Stranger**  **Silence**。


4. **Stranger 检查**: 默认  **Silence/RequestBox**。

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
* 动作：将 `SourceDID` 的所有 Bindings、Tags、Grants 移动到 `TargetDID`，然后标记 `SourceDID` 为 `Merged/Deleted`。



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

    def block_contact(self, did: DID): pass
    
    # ==========================
    # 3. 联系人管理 (User/Agent 调用)
    # ==========================
    def import_contacts(self, contacts: List[ImportSchema]) -> ImportReport:
        """
        批量导入。自动执行 "策略 A" (升级 Shadow Contact)。
        """
        pass

    def merge_contacts(self, target_did: DID, source_did: DID):
        """
        手动合并两个 DID。source 的 binding/grant/group/tag 迁移到 target，
        但 source DID 不删除，而是登记为 target 的 alias/tombstone，使历史
        MsgObject 和旧 session peer DID 仍可通过 resolve_canonical_did 解释。
        """
        pass

    def update_contact(self, did: DID, **kwargs):
        """
        修改备注、标签、分组等。
        """
        pass

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
* `DID_X.access_level` 升级为 `Friend` (假设导入即信任)。




4. **结果**: 之前的对话记录保留，联系人变为“熟人”，后续消息正常提醒。

---

## 6. UI/UX 建议

1. **影子标记**: 在消息列表中，Shadow Contact 的头像应有视觉区分（如灰色边框、虚线或问号角标），提示用户“未保存”。
2. **临时倒计时**: 对于 `Temporary` 权限的会话，标题栏显示倒计时（e.g., "临时会话：剩余 3 小时"）。
3. **来源展示**: 联系人详情页应展示来源（"来自 Telegram 自动添加" 或 "导入自 Google Contacts"）。