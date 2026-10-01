# Self-Host Group 设计 v2

- 版本：v2.0 草案，2026-09-30；同日对照 IM 业界实践评审后修订（序号与同步、消息关系、Session 记录、信任模型等）
- 状态：目标契约。除 §12「实现现状」明确标注的部分外，本文描述的是目标设计，不代表已经实现。
- 取代：[Self-Host-Group.md](<./Self-Host-Group.md>)（下称 v1）。v1 中与本文冲突的内容以本文为准；从 v1 迁出的主题见 §13。
- 上游：
  - [BuckyOS Self-host Group 的应用扩展架构设计](<./BuckyOS Self-host Group的应用扩展架构设计.md>)（下称「扩展架构」）
  - [Message Center.md](<./Message Center.md>)，尤其是 §2.3.1 Session inbox 地址
  - [Session State and Action Log.md](<./Session State and Action Log.md>)
  - cyfs-ndn《CYFS 标准对象》§16 MsgObject（`cyfs-ndn/doc/CYFS Protocol/CYFS 标准对象.md`，下称「MsgObject 规范」）
- 目标读者：Message Center / Group Service 开发者、Message Hub 开发者、群应用开发者。

## 0. 相对 v1 的主要变化

| # | v1 | v2 | 原因 |
|---|---|---|---|
| 1 | 群只有一个 GROUP_INBOX；subgroup 是父群内的成员子集 | 群由多个 **Group Session** 组成。`group_did/session_id` 是完整的寻址与权限单元，每个 Session 有自己的成员列表，未声明时继承群成员列表。Session 还可以加入群外的 **Session Guest**，因此 Session 成员列表可以是群成员的超集。subgroup 并入 Session | 对齐 Message Center 的 `$did/session_id` inbox 改造，以及扩展架构 §2.3 |
| 2 | 群消息地址前后矛盾（§2.7 与 §8#13） | 统一为 `from=actor, to=[group_did]`，目标 Session 由 MsgObject 的专用字段 `to_session` 指定 | 与 Message Center §3.4、扩展架构 §2.4 一致；`thread.topic` 只是语义 hint，不能做路由键 |
| 3 | push（逐成员 DeliveryRecord）与 pull 两种写法并存 | 原生成员以 pull 为准，每群一条按读者过滤的变更流；push 只用于 tunnel 成员和 host 本地成员的投影 | 读取权在成员从 host 获取时判定；避免逐 Session 轮询 |
| 4 | GroupDoc、GroupSettings 和若干 policy 分处多地 | Group Configuration 是唯一的规则来源，带 `schema_version`，是对外平台契约；GroupDoc 只是公开摘要。Session 本身是运行时记录，配置只放 Session 模板 | 扩展架构 §4.4；工单类场景的 Session 数量大、变化频繁 |
| 5 | 没有扩展点 | 定义 L1 配置、L2 面板、L3 Hook 点、L4A 访问路径、L4B 协议边界 | 扩展架构 §3–§8 |
| 6 | 角色直接等于权限 | 角色映射到可配置的能力集合；区分原生权限、动态许可和持久化变更 | 扩展架构 §2.5、§6.5 |
| 7 | API 带 `actor_did`、`host_owner` | 操作者只取自认证上下文；`group_did` 全局唯一，不再需要 `host_owner` | 安全 |
| 8 | 嵌套群递归展开、收益归属、公开成员双向证明列为第一版必须实现 | 迁出到独立的 DID Collection 设计，第一版不做 | 最小协议集 |
| 9 | 未来做多 host 共识 | 同一 Group 小规模多副本（主从），群主保留控制权 | 扩展架构 §10.2 |
| 10 | 消息按发送方的 `created_at_ms` 排序 | host 在接受时分配 `group_seq` / `session_seq`；游标、历史可见区间、回执都基于序号 | 发送方时钟不可信，按时间做游标会漏消息 |
| 11 | 没有编辑、撤回、回应、提及 | MsgObject 增加 `relates_to`（编辑 / 撤回 / 回应 / 话题）与 `mentions`；管理员可以删帖（§2.6） | 这些是互操作格式，越晚补代价越大 |
| 12 | 信任关系未说明 | self-host 即全听群主的；需要强证明时用 MsgObject 签名（§1.5） | 明确成员能相信什么 |

---

## 1. 定位与范围

### 1.1 Self-host Group 是什么

Self-host Group 是拥有独立 Group DID 的标准群，由群主所在 Zone 的 Message Center 托管。托管方（host Zone）是该群全部状态的唯一权威：成员资格、Session、配置、消息归档、消息顺序和访问判定都在 host 上完成。

它由两部分组成：

- **Group DID 实体**：群本身的身份。它是群消息的 `to`，也是群事件的发布者。DID Document 公开群的 host 和服务入口。
- **Self-host Group Service**：Message Center 内置的群服务默认实现，负责执行群规则、承载 Session 和消息。代码中的 GroupMgr 是其中的成员与配置模块。

Message Hub 是系统默认聊天界面，只是 Group Service 的一个客户端，应用可以不使用它（扩展架构 §1）。

本文的「群」指标准群：有群主、有管理员、有成员管理，由配置决定加入流程、可见会话、读取范围和发言权限。没有群主、成员地位平等的多人群发会话（扩展架构 §2.2 中的 mutual user session）不在本文范围内，它只是点对点消息的多目标投递（Message Center §4.3）。

### 1.2 设计原则

1. **最小协议集 + 最小默认实现。** 群服务只覆盖标准群必需的能力，业务逻辑通过 §7 的扩展点承接。
2. **host 单权威。** 第一版每个群只有一个 host，成员之间不做共识。
3. **Session 是隔离单元。** `group_did/session_id` 是独立的寻址、成员和权限单元。同一群的不同 Session 互不授权。
4. **订阅不等于读取权。** 成员能看到哪些 Session、读到哪些消息，由 host 在成员获取时判定。已经获取的副本不因后续撤权而回收。
5. **顺序由 host 决定。** 消息顺序、同步游标、历史可见区间和回执只使用 host 分配的序号，不使用发送方声明的时间。
6. **单向依赖。** 业务能力可以依赖群服务，群的基础聊天不依赖面板等业务能力。Hook 是例外，它的依赖由开发者显式声明（§7.3）。
7. **协议与默认实现分离。** §11.1 列出的协议能力是独立实现之间的互操作边界；存储布局、RPC 和配置格式属于默认实现。

### 1.3 非目标

- 不定义多 host 共识，不要求群消息在成员 Zone 上形成一致的可写副本。
- 不把联系人分组（`Contact.groups`）升级为群。
- 不定义收益结算、版权登记和递归 DID 集合（见 §13）。
- 不定义 UI 交互细节。
- 不做端到端加密。host 能看到全部明文（§1.5）。
- 输入状态、在线状态等临时状态不进入群消息流；跨 Zone 的群 Runtime State 第一版不做。
- 第一版没有按时间自动过期的保留策略，也没有 host 侧的群内全文检索（成员检索自己已获取的副本）。

### 1.4 术语

| 术语 | 含义 |
|---|---|
| Group DID | 群的身份 DID，可以是形如 `<group_id>.<zone_id>` 的二级 DID，也可以是用户自备的一级 DID |
| Host Zone | 托管该群的 Zone，即群主控制的 Zone/OOD |
| Group Session | 群内的一个会话，由 host 权威管理。具名 Session 的地址是 `group_did/session_id`，默认 Session 的地址是裸 `group_did` |
| Session 记录 | host 上描述一个具名 Session 的运行时记录：成员列表声明、规则、生命周期（§2.2.5） |
| Session 模板 | 配置中预先定义的成员列表声明与规则，创建 Session 时引用（§3.2） |
| Session 成员列表 | 有权参与某个 Group Session 的 DID 集合，由群成员部分和 Session Guest 两部分组成；群成员部分未声明时继承群成员列表 |
| Session Guest | 不是群成员、只被加入某个具名 Session 的 DID，例如客服工单中的客户 |
| 有效成员 | 某一序号处对某个 Session 实际生效的参与者，包括群成员和 Session Guest（§2.3.5） |
| `group_seq` / `session_seq` | host 分配的群内全局序号 / Session 内消息序号（§5.1） |
| 可见区间 | 某个参与者在某个 Session 中作为有效成员的 `group_seq` 区间，用于计算历史可见范围（§2.3.5） |
| 成员周期（epoch） | 一个 DID 每次进入群 `Active` 状态时开始一个新周期，Session 成员表记录绑定到具体周期（§2.3.3） |
| 变更流 | host 为每个读者提供的、按读者可见性过滤后的群变化序列（§5.4.1） |
| 消息关系 | 一条消息对另一条消息的编辑、撤回、回应或话题归属，由 `MsgObject.relates_to` 表达（§2.6） |
| Group Configuration | 群的规则、展示内容与扩展声明，是 Message Center 对外的稳定契约 |
| 本地 Session 投影 | 某个 owner 在自己的 Message Center 中看到的会话视图，键为 `(owner, session_id)`（Message Center §5） |
| Hosted / Joined Group | 当前 Zone 托管的群 / 当前 Zone 用户只是成员的群 |
| Application Service | 提供面板、Hook 响应或业务数据的应用服务 |

「Group Session」和「本地 Session 投影」处在两个层次：前者是 host 上的权威对象，后者是每个成员各自的视图。两者通过 `SessionStateRef { authority_did: group_did, session_key }` 关联（§2.2.4）。下文单说「Session」时，均指 Group Session。

### 1.5 信任模型

self-host 的含义是**全听群主的**：

- host 由群主控制，是群内全部事实的权威。成员资格、Session、消息内容、消息顺序和可见范围都以 host 为准。
- 成员从 host 读到的消息，默认由群主背书，包括 `from`、内容和顺序。成员 Zone 不需要、也无法独立验证这些事实。
- 群主可以读取全部 Session（§2.3.4）。不做端到端加密。
- 需要强证明时，即证明某条消息确实由 `from` 创建、host 没有伪造或篡改，发送者以 JWT 形式提交 MsgObject（CYFS 标准对象统一的签名方式，见 MsgObject 规范 16.5）。这是协议级能力。签名后的消息任何人都可以独立校验，host 无法伪造或改动；签名不影响 ObjectId。
- host 收到 JWT 形式的消息必须校验签名，校验失败就拒绝，不能降级为 JSON 形式接受。校验通过的，host 保存 JWT 原文，读取时原样提供（§5.1）。如果读者拿到的只有 JSON 形式，这条消息只由群主背书。
- 群服务不要求成员签名，因为外部平台成员（shadow endpoint DID）无法签名。
- Action Log 事件由群发布（`from = group_did`），同样由群主背书。

---

## 2. 实体与寻址模型

### 2.1 Group DID

- 默认生成形如 `<group_id>.<zone_id>` 的二级 DID。用户也可以登记自己控制的一级 DID，登记时 host 必须验证该 DID Document 的 controller 已授权本 host。
- DID Document 公开以下信息：
  - 实体类型 `group`；
  - host Zone/OOD；
  - controller；
  - 群服务入口，即 CYFS 语义路径前缀（§11.1）；
  - 可选的公开资料。
- **DID 控制权与群治理权分开。**
  - DID 控制权包括更新 DID Document、迁移 host、转让控制权，只按 DID Document 的 controller 验证。
  - 群治理权包括邀请、审批、修改配置、管理 Session，由 Group Configuration 中的角色能力决定（§3.3）。
  - 二者可以由同一人持有，但不能互相推导。
- Group DID 可以出现在任何接受 DID 的位置，例如 `to`、权限主体、作者。
- **Session 不是 DID**，不能作为 `from`/`to`、作者或权限主体出现。如果一个会话需要独立身份、独立的成员确认或独立的 host，应创建新的 Group DID。Message Center §5.5 所说的「子群」就是指这种独立群。
- 群删除后，host 为 `group_did` 保留墓碑，不再用它创建新群（§6.3）。

### 2.2 Group Session

#### 2.2.1 地址

Group Session 的地址直接使用 Message Center 的 `MailboxAddress`（Message Center §2.3.1）：

```text
group_did              默认 Session（session_id = NULL）
group_did/session_id   具名 Session
```

- 每个群恰好有一个默认 Session，随群创建，不能删除。群级事件（入群、退群、修改群资料）发布在默认 Session。
- 具名 Session 的 `session_id` 遵守 MailboxAddress 的规则：长度 1–200 个字符，不含首尾空白和控制字符，不能是 `.` 或 `..`。以 `_` 开头的 ID 保留给系统使用。
- `session_id` 默认由 host 生成不透明 ID；创建者也可以指定（例如配置生成器需要固定的 `announcements`）。它在群内唯一，创建后不能改名，**删除后也不能复用**（§6.4）。给人看的名称是 Session 标题（§2.2.4）。
- host 上该 Session 的消息记录是 `GROUP_INBOX` 记录：`owner = group_did`，`session_id` 等于该 Session 的 ID。对应的 RBAC 资源是 `obj://msg-center/group_inbox/<编码后的地址>`。

#### 2.2.2 为什么 Session 是完整实体

Message Center 已经把 `did + session_id` 作为独立的 inbox：裸 DID 与任意具名会话互不读取，权限按精确地址授予（Message Center §2.3.1）。群沿用这一设计，每个 Session 都是完整的隔离单元，各自拥有：

- 独立的成员列表（§2.3）；
- 独立的规则：发言权限、历史可见范围、Hook 绑定、面板（§3.2）；
- 独立的共享状态：标题、说明、公告（§2.2.4）；
- 独立的消息序号、归档、未读状态和回执。

能访问某个 Session，不意味着能访问同群的其它 Session；能访问默认 Session，也不意味着能访问具名 Session。

#### 2.2.3 Session 示例

| 场景 | 成员列表 | 发言规则 |
|---|---|---|
| 默认群聊 | 继承群成员 | 所有成员 |
| 公告频道 | 继承群成员 | 仅管理员发言，所有成员可以回应 |
| 管理员频道 | 按角色：Owner、Admin | 所有 Session 成员 |
| 项目小组 | 显式列出部分成员 | 所有 Session 成员 |
| 客服工单（由 L1/L3 应用创建，或由客户发起，§6.5.2） | 显式列出客服（群成员），加上客户（Session Guest） | 所有 Session 成员，可加 Hook |

#### 2.2.4 Session 的共享状态

Session 的标题、说明、公告等描述性字段属于 `SessionSharedState`；成员在本 Session 中的昵称属于 `SessionMemberState`。两者都按 [Session State and Action Log.md](<./Session State and Action Log.md>) 管理。对 Group Session：

```text
SessionStateRef = { authority_did: group_did, session_key: <该 Session 的规范 MailboxAddress 字符串> }
默认 Session：session_key = group_did
具名 Session：session_key = group_did/<按 MailboxAddress 规则编码的 session_id>
```

- 一个 Group Session 在所有对外可见的键中只有一种写法，即它的规范 MailboxAddress 字符串。`SessionStateRef.session_key` 和成员侧本地 Session 键（§5.5）都使用它。只有 host 数据库的 `session_id` 列按 Message Center 的约定用 NULL 表示默认 Session。
- 权威是 host 上的 Group Service；成员 Zone 只保存已确认的副本。
- Session 的**规则**（成员列表声明、发言、读取、Hook）属于 Session 记录与配置中的模板（§2.2.5、§3.2），不放进 `SessionSharedState.extensions`，避免有人通过状态 patch 改写权限。

#### 2.2.5 Session 记录

具名 Session 是 host 上的运行时记录，不属于 Group Configuration。创建、归档一个 Session 不产生配置 revision；工单类场景可以有成千上万个 Session。

```rust
pub struct GroupSessionRecord {
    pub group_did: DID,
    pub session_id: String,
    pub template: Option<String>,           // 引用配置中的 session_templates（§3.2）
    pub membership: SessionMembership,      // 创建时取自模板，之后可以单独修改（§2.3.1）
    pub rule_overrides: SessionRulesPatch,  // 只记录与模板不同的规则字段
    pub lifecycle: SessionLifecycle,        // Active | Archived | Deleted（墓碑）
    pub revision: String,
    pub created_by: DID,
    pub created_at_ms: u64,
}
```

- 生效规则 = 模板规则（未引用模板时用内置默认规则）叠加 `rule_overrides`。修改模板会影响所有引用它的 Session；配置不能删除仍被引用的模板。
- 修改 Session 记录要带 `expected_revision`，并产生 Session 级 Action Log（§6.4）。
- 默认 Session 没有单独的记录，它的规则是配置中的 `default_session`。
- `Deleted` 是墓碑：Session 的消息和成员表已经删除，只保留 ID，防止复用。

### 2.3 Session 成员列表

Session 成员列表由两部分组成：

```text
Session 成员列表 = 群成员部分 ∪ Session Guest
```

- **群成员部分**：从群成员中选出的参与者，由 `SessionMembership` 声明，只能是群成员的子集。
- **Session Guest**：不是群成员、只被加入这个 Session 的 DID。有了它，Session 成员列表可以是群成员的超集。

#### 2.3.1 群成员部分

每个 Session 在记录中声明群成员部分的来源：

```rust
pub enum SessionMembership {
    /// 缺省值：等于群的成员列表，随群成员变化自动更新。
    Inherit,
    /// 群内具有指定角色的成员，随角色变化自动更新。
    Roles(Vec<GroupRole>),
    /// 显式列表，成员逐个加入（见 §2.3.3 的 SessionMembershipRecord）。
    Explicit,
}
```

- 不声明即为 `Inherit`。默认 Session 固定为 `Inherit`，不能修改。
- 允许 Guest 的 Session（`allow_guests = true`）不能使用 `Inherit`。否则新入群的成员会在不知情时进入一个有外部人员的 Session。
- 群成员加入 Session 不需要新的签名。成员同意加入群时签署的 `GroupMemberProof` 已经覆盖群内的 Session。

#### 2.3.2 Session Guest

Guest 不入群，只加入具体的 Session。典型场景是客服工单：客户只被加入自己的工单 Session，看不到群的其它部分。

- Guest 只能加入具名 Session，不能加入默认 Session。Session 是否接受 Guest 由其规则决定（`allow_guests`，§3.2）。
- 加入需要 Guest 本人同意：Guest 签署作用域为该 Session（或为本次请求新建的 Session）的 proof（§2.4）。
- 加入有两种方式：由群内有权限的人邀请（§6.5.1），或由 Guest 从群公开的入口发起请求、host 为其新建 Session（§6.5.2）。
- Guest 只能在该 Session 内活动：按 Session 规则读取、发言、回应、编辑和撤回自己的消息，修改自己在该 Session 的昵称，退出该 Session。
- Guest 没有任何群级能力（§3.3）：
  - 看不到默认 Session、群里的其它 Session 和群成员列表；
  - 不能创建 Session，也不能邀请他人。
- 含 Guest 的 Session 对全体参与者显示「含外部成员」标识（§5.3）。
- 同一个 DID 可以是多个 Session 的 Guest。每个 Session 分别确认，互不授权。
- 被群封禁（`blocked`，§2.4）的 DID 同时失去全部 Session Guest 资格。
- Guest 之后正式入群时，其 Guest 记录转为当前成员周期的 `Included` 记录，在该 Session 的可见区间保持连续。

#### 2.3.3 Session 成员表

单个 DID 与 Session 的关系保存在 host 的 Session 成员表中。这张表是运行时状态，不属于配置，因此频繁的成员增减不会产生配置 revision：

```rust
pub struct SessionMembershipRecord {
    pub group_did: DID,
    pub session_id: Option<String>,    // None 为默认 Session
    pub member_did: DID,
    pub kind: SessionParticipantKind,  // GroupMember | Guest
    pub epoch: u32,                    // 见下文
    pub state: SessionParticipation,   // Invited | Included | Left | Removed
    pub proof_id: Option<ObjId>,       // Guest 必填，指向 Session 作用域的 proof
    pub since_seq: u64,                // 状态生效时的 group_seq
    pub actor: DID,                    // 执行该变化的操作者
}
```

- `epoch`：对 `GroupMember`，是该记录所属的群成员周期（`GroupMemberRecord.epoch`，§2.4）。成员退群后再入群会开始新周期，旧周期的记录全部失效。对 `Guest`，是该 DID 在这个 Session 的第几次 Guest 参与。
- `Invited` 只用于 Guest，表示正在等待 Guest 提交 proof。
- 对群成员（只看当前周期的记录）：
  - 在 `Explicit` Session 中，`Included` 记录定义成员；
  - 在 `Inherit` / `Roles` Session 中，`Left` / `Removed` 记录定义例外。
- 对 Guest：总是通过 `Included` 记录加入，与 `SessionMembership` 的声明无关。
- 命名上注意区分：`SessionMemberState` 是 Session State and Action Log.md 中的成员昵称等展示状态，与本表无关。

#### 2.3.4 群主的完整视图

扩展架构 §2.5 规定群主掌握群消息和配置的完整视图。因此，群主对所有 Session 都有读取权，即使不在 Session 成员列表中。这种读取属于审计和管理访问：

- 群主不会因此出现在 Session 成员列表中；
- 系统也不为群主生成成员侧的未读投影；
- Message Hub 在 Session 信息中告知参与者「群主可以查看本 Session」。

管理员是否有同样的权限，由能力 `group.read_all` 决定（§3.3），不按角色名称推定。群主以外的人使用 `group.read_all` 读取非本人参与的 Session 时，host 写入审计记录，群主可以查看。

#### 2.3.5 有效成员与可见区间

host 为群内每个被接受的变化分配 `group_seq`，包括消息写入，以及成员、角色、Session 成员表和配置的变化（§5.1）。有效成员关系按 `group_seq` 判定，不按时间。

在 `group_seq = n` 处，DID d 是 Session s 的有效成员，当且仅当满足以下任一条件：

1. **群成员路径**：d 在 n 处是群的 `Active` 成员，当前成员周期为 e，并且满足 s 的 `SessionMembership` 声明：
   - `Inherit`：没有周期 e 的 `Left` / `Removed` 记录；
   - `Roles`：角色在列表中，并且没有周期 e 的 `Left` / `Removed` 记录；
   - `Explicit`：有周期 e 的 `Included` 记录。
2. **Guest 路径**：s 是具名 Session 且允许 Guest，d 有状态为 `Included`、proof 有效的 Guest 记录，并且 d 没有被群封禁。

由此可以得出：

- 被移出群的成员同时失去所有 Session 的群成员资格，不需要逐个 Session 修改。被封禁的 DID 同时失去 Guest 资格。
- `Roles` 型 Session 的成员随角色变化自动更新。
- 重新入群的成员开始新周期，上一周期在 Session 中留下的 `Left` / `Removed` 记录不再生效。
- 每个参与者在每个 Session 有一组**可见区间** `[from_group_seq, to_group_seq)`，即他作为有效成员的各段时期。`history = FromJoin` 时，一条消息对读者可见，当且仅当它的 `group_seq` 落在读者的某个可见区间内。因此重新入群的成员能看到自己以前在群时的消息，看不到不在群期间的消息。

host 物化并保存每个参与者的可见区间，并保留产生这些区间的成员变化记录（来自 GroupEvent / Action Log）。

### 2.4 群成员与 Session Guest 的确认

**群成员状态机**只表达群成员资格，沿用 v1 §4.2 的 `GroupMemberRecord`，简化为：

```text
Invited → (PendingAdminApproval) → Active → Left / Removed
Invited → Expired / Revoked
PendingAdminApproval → Rejected
```

- `Left`、`Removed`、`Rejected`、`Expired`、`Revoked` 之后，可以重新被邀请或重新申请，进入新的 `Invited` / `PendingAdminApproval`；被封禁（`blocked`）的 DID 除外。审批只能推进 `PendingAdminApproval`，不能直接复活。
- `GroupMemberRecord` 增加 `epoch`（每次进入 `Active` 加一）和 `entity_kind`（User / Agent / Device，取自成员的 DID Document）。

**Session Guest 状态机**记录在 Session 成员表中（§2.3.3）：

```text
Invited → Included → Left / Removed
```

**moderation 标记**从成员状态中移出，独立保存。它作用于群范围内的任何 DID，包括 Session Guest：

- `muted_until_ms`：禁言。期间不能发言，仍然可以读取。
- `blocked`：封禁。禁止再次申请、被邀请或作为 Guest 加入，直到解除。

**proof。** 群成员和 Session Guest 使用同一种 proof 对象（沿用 v1 §4.3 的 `GroupMemberProof`），按作用域区分：

```rust
pub enum MemberProofScope {
    Group,                    // 加入群；role 为成员同意接受的群角色上限
    Session(String),          // 以 Guest 身份加入指定的具名 Session；不带群角色
    SessionRequest(String),   // 以 Guest 身份加入 host 为本次请求新建的 Session；参数是请求 ID（§6.5.2）
}
```

建立关系之前，proof 必须全部通过以下校验：

- `signer` 是 `member_did`，或是 `member_did` 的 DID Document 授权的 key/agent；
- 签名按签名者的 DID Document 验证；
- `group_did`、`member_did`、作用域和 `role` 与当前的邀请或申请一致；
- `nonce` 未被使用过，proof 未过期。

**外部平台用户。** `did:msgtunnel:*` 这类 shadow endpoint DID 无法签名。外部平台用户无论是群成员还是 Guest，其同意都以对应 tunnel 实例给出的接入证据代替签名：记录 `attested_by = transport_did`，以及平台侧的来源事件（例如用户在平台上主动发起会话或接受邀请）。这是签名要求的唯一例外（待确认，§14）。

**角色来源。**

- 群角色为 `Owner`、`Admin`、`Member`。v1 的 `Guest` 角色取消，由 Session Guest 取代。
- 主动申请入群、通过邀请链接入群，都只能得到 `Member`；更高的角色只能通过邀请或 `update_member_role` 授予。
- proof 中的 `role` 只表示成员同意接受的角色上限，host 不能据此提升成员角色。

**其它约束。**

- 同 Zone 用户的 proof 可以由本 Zone 在用户授权下自动构造，但必须是真实可验证的签名，不能用占位字符串代替。
- 第一版的 `member_did` 必须是单体实体 DID，即用户、Agent 或设备。以 Group DID 作为成员（嵌套群）见 §13。

### 2.5 群消息的地址语义

```text
MsgObject {
  from:        actor DID           // 实际发送者；外部平台成员用 shadow endpoint DID
  to:          [group_did]         // 必须只有一个目标，即群
  kind:        group_msg
  to_session:  session_id          // 省略时表示默认 Session
  thread:      { topic?, reply_to?, correlation_id? }   // 语义线索，不参与路由
  relates_to:  { rel, target, key? }?                   // 消息关系（§2.6）
  mentions:    { dids, all }?                           // 提及（§2.6）
}
// 需要强证明时，以该对象为 claims 的 JWT 形式提交（§1.5）
```

字段的协议定义见 MsgObject 规范。群的规则如下：

- `from` / `to` / `to_session` 在消息创建时确定，任何环节都不改写（Message Center §3.4）。不同读者读到的同一条消息，`to` 都仍然是群。
- `to_session` 是发送方声明的目标 Session，host 严格按它路由：
  - 如果 `to_session` 指向不存在的 Session，或者发送者不是该 Session 的有效成员，**拒绝**写入，而不是落入默认 Session。落入默认 Session 会把原本面向小范围的内容暴露给更多人。
  - 对非有效成员，「Session 不存在」和「无权访问」返回同一个 `not-found`（§4.1），不泄露 Session 是否存在。
  - host 不会根据未知的 `to_session` 自动创建 Session。
- `thread.topic` 只是语义 hint，host 不用它路由。过渡期内（beta 2.2），host 拒绝带 `thread.topic` 但不带 `to_session` 的群消息（`missing-to-session`），防止旧客户端把面向具名 Session 的消息写入默认 Session。
- 回复群消息时使用同一个 `to` 和同一个 `to_session`。私聊发送者必须是用户的显式动作。
- `created_at_ms` 是发送方声明的时间，只用于展示。排序和可见性使用 host 序号（§5.1）。
- 群事件（Action Log，见 Session State and Action Log.md §4）由群发布：`from = group_did`、`to = [group_did]`、`kind = event`。Session 级事件带对应的 `to_session`；群级事件发布在默认 Session。

### 2.6 消息关系与提及

编辑、撤回、回应和话题都是一条新的 MsgObject，用 `relates_to` 指向原消息；提及用 `mentions` 表达。协议定义见 MsgObject 规范，本节是群内的规则。关系消息的 `kind` 与原消息相同（`group_msg`），走同一写入路径（§5.2），`post` Hook 同样适用。

**通用约束。**

- `relates_to.target` 必须是同一 Session 中已经接受的消息，否则拒绝（对非有效成员仍是 `not-found`）。
- `thread.reply_to` 应当指向同一 Session 的消息。指向读者不可读的消息时，客户端只显示「无法查看的消息」。

**编辑（`edit`）。**

- `from` 必须等于原消息的 `from`，并且在 Session 规则的 `edit.edit_window_ms` 内。
- 只能编辑普通消息，不能编辑关系消息和 Action Log。
- 每次编辑都指向原消息，不指向上一次编辑；以 `session_seq` 最大的编辑为准。

**撤回与删帖（`redact`）。**

- 本人在 `edit.recall_window_ms` 内可以撤回自己的消息；具有 `message.redact_any` 能力的操作者可以随时删除任意消息，包括 Guest 的消息。删除他人消息时，撤回消息的 `content` 可以写明原因。
- host 的处理：
  - 把原消息标记为已撤回，并删除原消息及其编辑的 MsgObject 正文（没有其它引用时），不保留可恢复副本；
  - 之后的列表只返回占位，按 ObjectId 读取原消息返回 `not-found`；
  - 原消息的回应随之失效。
- 撤回消息本身经变更流传到成员 Zone。成员 Zone 隐藏原消息，并应当删除本地副本。
- tunnel 成员：由 tunnel 在平台上删除对应消息；平台不支持时，发送文字说明。
- 这是尽力而为的传播：已经离开群、不再同步的成员副本无法撤回。

**回应（`reaction`）。**

- 受 Session 规则 `react` 约束。例如公告频道可以只允许管理员发言，但所有成员都能回应。
- 同一个 `(from, target, key)` 只记一次。取消回应就是撤回这条回应消息。

**话题（`thread`）。**

- `target` 是话题的根消息。话题内的消息与 Session 中的其它消息共用成员、规则和序号。

**提及（`mentions`）。**

- `mentions.dids` 中不是有效成员的 DID 被忽略。
- `mentions.all` 需要 `session.mention_all` 能力，否则拒绝（`mention-all-not-allowed`）。
- 成员 Zone 依据该字段计算 @ 提醒，不解析正文。成员在本地静音 Session 后，是否仍提示 @ 由个人偏好决定。

---

## 3. Group Configuration

### 3.1 定位

Group Configuration 是 Message Center 对应用承诺长期兼容的平台契约（扩展架构 §4.4、§10.1）。

- **唯一来源。** 群规则只在配置中定义。GroupDoc 中的策略只是从配置派生出的公开摘要，不能单独修改。具名 Session 本身是运行时记录（§2.2.5），配置只定义它们引用的模板。
- **版本规则。** 配置带 `schema_version` 和 `required_features`。新版本可以增加字段，但不能改变已有字段的语义；改变默认值也视为语义变化。
- **未知内容。** 未知字段要忽略并原样保留。`required_features` 中出现实现不认识的条目时，拒绝加载并报告，不能按默认值静默运行。需要旧实现不能忽略的新语义时，新增一个 feature 名称并写入 `required_features`。
- **修改契约。** 每次修改产生新的 `revision`。修改请求必须带 `expected_revision` 和幂等键，与 SessionSharedState 的修改契约一致。
- **审计。** 修改配置会产生 Action Log（`entity.config_changed`）；公开字段的 before/after 按可见性过滤。

beta 2.2 的实现可以不兼容旧数据，但配置规范从第一版起就要带版本字段。

### 3.2 结构

以下是逻辑结构，字段名在实现时冻结：

```rust
pub struct GroupConfiguration {
    pub schema_version: u32,
    pub required_features: Vec<String>,
    pub revision: String,

    pub profile: GroupProfileConfig,                         // 名称、头像、说明
    pub membership: MembershipConfig,                        // 加入流程、成员列表可见性、Guest 入口
    pub roles: RoleCapabilities,                             // 角色 → 能力（§3.3）
    pub default_session: SessionRules,                       // 默认 Session 的规则
    pub session_templates: BTreeMap<String, SessionTemplate>,// 具名 Session 的模板
    pub limits: GroupLimits,                                 // 规模与频率上限（§6.7）
    pub display: GroupDisplayConfig,                         // 群公告、固定链接、常用文件、展示成员
    pub access: AccessConfig,                                // 允许的客户端（§4.3）
    pub extensions: ExtensionConfig,                         // 面板与 Hook 声明（§7）
}

pub struct MembershipConfig {
    pub join_policy: JoinPolicy,                   // InviteOnly | RequestAndApprove | Open
    pub member_list_visibility: Visibility,        // Members | AdminsOnly | Public
    pub guest_entry: Option<GuestEntryConfig>,     // 是否接受 Guest 主动发起请求（§6.5.2）
}

pub struct GuestEntryConfig {
    pub session_template: String,                  // 新建 Session 使用的模板，必须 allow_guests 且不是 Inherit
    pub max_open_per_guest: u32,                   // 同一 Guest 同时处于 Active 的请求 Session 上限
}

pub struct SessionTemplate {
    pub membership: SessionMembership,             // 缺省 Inherit（§2.3）
    pub rules: SessionRules,
}

pub struct SessionRules {
    pub post: PostRule,                            // AllParticipants | Only(Vec<ParticipantKind>) | Nobody
    pub react: PostRule,                           // 回应权限，缺省 AllParticipants
    pub history: HistoryVisibility,                // FromJoin | All
    pub allow_guests: bool,                        // 缺省 false，默认 Session 恒为 false；为 true 时成员列表不能是 Inherit
    pub edit: EditRule,                            // 编辑与撤回窗口
    pub receipts: ReceiptVisibility,               // Hidden | Count | Readers，缺省 Hidden（§5.6）
    pub slow_mode_ms: Option<u32>,                 // 同一发送者两次发言的最小间隔
    pub hooks: Vec<HookBinding>,                   // L3（§7.3）
    pub panels: Vec<String>,                       // 引用 extensions.panels 中的 id（§7.2）
}

pub struct EditRule {
    pub edit_window_ms: Option<u64>,               // None 为不限，Some(0) 为禁止编辑
    pub recall_window_ms: Option<u64>,             // 本人撤回窗口；None 为不限
}

pub struct GroupDisplayConfig {
    pub announcement: Option<String>,
    pub pinned_links: Vec<PinnedLink>,
    pub pinned_files: Vec<ObjectRef>,              // 引用 FileObject，不内联内容
    pub featured_members: Vec<FeaturedMember>,     // 仅用于展示
}
```

说明：

- `display` 只用于展示，**不是授权**。`featured_members` 中标为「官方客服」的成员，并不因此获得管理权（扩展架构 §9.5）。
- `ParticipantKind` 取 `Owner | Admin | Member | Guest`，其中 `Guest` 指 Session Guest。例如工单 Session 可以设为 `AllParticipants`，客户咨询频道可以设为只允许 Admin 和 Guest 发言。
- `slow_mode_ms` 对 Admin 及以上不生效。
- Session 的标题、说明和 Session 公告属于 SessionSharedState（§2.2.4），不放在配置中。
- 被引用的链接和文件由提供方负责其可用性。引用失效不影响群聊。

### 3.3 角色与能力

角色是能力的命名集合，映射关系写在配置的 `roles` 中。默认映射如下：

| 能力 | Owner | Admin | Member |
|---|---|---|---|
| `group.update_profile` / `group.update_config` | ✓ | ✓ | |
| `group.invite_member` / `group.approve_member`（含创建邀请链接） | ✓ | ✓ | |
| `group.remove_member` / `group.moderate` | ✓ | ✓ | |
| `group.update_role` | ✓ | | |
| `group.read_all`（读取所有 Session） | ✓ | | |
| `message.redact_any`（删除任意消息） | ✓ | ✓ | |
| `session.create` | ✓ | ✓ | 可配置 |
| `session.manage`（成员、规则、归档、删除） | ✓ | ✓ | 仅限自己创建的 Session |
| `session.invite_guest`（邀请或移除 Session Guest） | ✓ | ✓ | 仅限自己创建的 Session，可配置 |
| `session.update_shared_state`（标题、说明、公告） | ✓ | ✓ | 可配置 |
| `session.mention_all`（@所有人） | ✓ | ✓ | 可配置 |
| `session.post` / `session.read` | 按 Session 规则 | 按 Session 规则 | 按 Session 规则 |

约束：

- Owner 的能力不能通过配置削减。转让 Owner 需要 DID 控制权（§2.1）。
- `session.post` / `session.read` 只是「可以参与」的前提，最终能否发言或读取，还取决于有效成员资格和 Session 规则（§4.1）。
- 编辑、撤回自己的消息不需要单独的能力，只受 Session 规则的时间窗约束。

**Session Guest 的能力**是固定的，不通过 `roles` 配置：

- 在自己作为 Guest 的 Session 中，按 Session 规则读取、发言、回应，编辑和撤回自己的消息；
- 修改自己在该 Session 的 SessionMemberState（例如昵称）；
- 退出该 Session；
- 读取群的公开资料（名称、头像），以便界面展示这是哪个群的会话；
- 查看该 Session 中与自己对话的参与者：默认包括该 Session 的显式参与者（有 `Included` 记录的群成员和其他 Guest），以及在该 Session 发过言的参与者的公开资料；不展开 `Roles` 带来的全部群成员。

Guest 不具有任何群级能力。

### 3.4 原生权限、动态许可与持久化变更

- **原生权限**：由配置、成员记录、Session 记录和 Session 成员列表直接得出的权限。
- **动态许可**：L3 Hook 对单次请求给出的放行，例如「游戏等级达标即可发言」。它只对这一次请求有效，不写回配置，也不改变原生权限。
- **持久化变更**：应用通过正常接口修改成员角色、Session 成员、Session 记录或配置。这类变更带操作者、revision 和 Action Log，应用失效后依然有效；要回收，也走同样的接口。

三者不能混用。Hook 不可用时，动态许可失效，原生权限不受影响（扩展架构 §6.5）。

### 3.5 一次性创建（L1）

扩展架构 §4 要求配置生成器创建完群即可退出。为此，Group Service 提供一次性创建接口：

```text
group.create(profile, configuration, sessions?, invitations?, idempotency_key)
  -> { group_did, revision }
```

该接口在一个事务内完成以下工作：

- 创建 Group DID 和 DID Document；
- 写入配置和 Owner 的成员记录；
- 创建默认 Session，以及声明的具名 Session 记录（含显式成员）；
- 发出邀请。

用同一个幂等键重复调用，返回同一结果。创建之后，应用可以用 `group.apply_config(expected_revision, patch)` 修改配置，用 Session 接口（§11.2）增删 Session。

---

## 4. 访问控制

### 4.1 判定顺序

每个请求按以下顺序判定，任何一步拒绝即结束：

1. **认证**：确定操作者。RPC 请求从 verify-hub token 取得；跨 Zone 的 CYFS 请求通过验证 `cyfs-original-user` 和 `cyfs-proofs` 得到。请求体中自报的 DID 不能作为操作者。
2. **根权限**：与 DID Document 相关的操作只认 controller。
3. **成员资格**：操作者必须是群的 `Active` 成员；对 Session 内的操作，也可以是该 Session 的有效 Guest。申请入群、提交 proof、Guest 发起请求等少数操作除外。`Removed` 或 `blocked` 一律拒绝。
4. **访问路径**：请求经由的客户端是否被群允许（§4.3）。
5. **能力**：操作者的角色具有该操作所需的能力（§3.3）。Session Guest 只有 §3.3 列出的固定能力。
6. **Session 规则**：对 Session 内的读写，操作者必须是有效成员，满足 post / react / history / edit 规则、慢速模式，且不在禁言期内。
7. **频率限制**：按操作者和 Session 计数（§6.7），Agent 单独计数。
8. **Hook**：如果该 Session 的这项操作绑定了 Hook，按 §7.3 执行。

**不泄露存在性。** 对 Session 的操作，如果操作者不是该 Session 的有效成员，也没有 `group.read_all`，那么无论 Session 是否存在，都返回同一个 `not-found`。只有有效成员才会得到更具体的原因，例如 `session-archived`、`muted`、`post-not-allowed`。按 ObjectId 读取消息、Guest 提交 Session proof 也遵守这一规则。

**Hook 不能覆盖的规则：**

- 第 1–4 步的拒绝；
- `Removed` 或 `blocked` 状态；
- `from` 必须等于认证得到的操作者；
- DID 根权限；
- 第 7 步的频率限制。

Hook 只能在第 5–6 步的原生拒绝之上给出动态许可，并且只有当配置声明了该 Hook 可以放行时才行。

### 4.2 与 Message Center RBAC 的关系

群成员大多不是本 Zone 用户，因此群访问不能依赖 Zone RBAC 中 `users` 组的授权。分工如下：

- 群成员（包括外部 DID）的读写，由 Group Service 按 §4.1 判定。
- 本 Zone 的应用或 Agent 直接访问某个 Session 的 GROUP_INBOX 时（例如应用驱动工单 Session），既需要 RBAC 对精确资源 `obj://msg-center/group_inbox/<group_did>/<sid>` 的授权，也必须经过 §4.1 的判定。RBAC 授权不能替代群授权。
- 当前 `rbac_config.rs` 给 `users` 组授予了 `group_inbox/*` 的读写权限，需要收窄（§12.2）。

### 4.3 访问路径限制（L4A）

`AccessConfig.allowed_clients` 声明允许哪些客户端访问群：

```rust
pub enum AllowedClients {
    Any,                  // 默认值
    Only(Vec<AppId>),     // 例如只允许某个业务应用；Message Hub 也是一个 AppId
}
```

- 客户端身份取自认证上下文中的应用身份，不取自页面地址或请求参数。
- 跨 Zone 访问时，远端客户端身份的携带和验证方式还没有确定（§14）。在确定之前，`Only` 只对本 Zone 内的访问有效；远端成员的请求按配置选择拒绝或放行，不能假装已经完成验证。
- 限制访问路径后，如果自定义应用失效，用户能否改从 Message Hub 进入，取决于配置是否把 Message Hub 列为备用客户端（扩展架构 §7.2）。

---

## 5. 数据面

### 5.1 存储与序号

- `MsgObject` 按内容寻址，只存一份。以 JWT 形式提交的消息，同时保存 JWT 原文，按 ObjectId 读取时原样返回 JWT 形式。
- host 为每条被接受的群消息写一条 `GROUP_INBOX` 记录：`owner = group_did`，`session_id` 为目标 Session（默认 Session 为 NULL）。同一个 `msg_id` 在一个群内只有一条记录。
- **序号。** host 在接受消息的同一事务中分配以下值。它们属于记录，不写入 MsgObject：
  - `group_seq`：群内全局单调递增。消息、Action Log 事件，以及成员、角色、Session 成员表、Session 记录和配置的变化都占用一个值。用于计算可见区间（§2.3.5）和变更流（§5.4.1）。
  - `session_seq`：Session 内单调递增且无空洞，只分配给写入该 Session 的消息与事件。用于展示顺序、分页和已读水位（§5.6）。
  - `accepted_at_ms`：host 接受消息的时间，单调不减（时钟回拨时沿用上一个值）。GROUP_INBOX 记录和成员投影记录的 `sort_key` 取这个值，不取发送方的 `created_at_ms`。
- 新接受的消息的序号一定大于此前分配的所有序号，因此按序号分页的游标不会漏掉晚到的消息（例如经 Gateway 暂存后才转投的消息）。
- 撤回不回收序号：被撤回的消息保留序号和占位。
- 群消息**不能** `move_record`。目标 Session 写在 MsgObject 的 `to_session` 中，移动记录会造成记录与对象不一致。需要转移时，由有权限者在目标 Session 重新发送（可以引用原消息），并按需撤回原消息。
- 以下内容是 host 的权威状态，存放在 Message Center RDB 中，必须纳入备份（§9）：成员资格与成员周期、可见区间、配置、Session 记录与墓碑、Session 成员表、moderation 标记、邀请链接、已读水位、序号计数器。

### 5.2 写入

```text
1. 认证：确定 actor；校验 MsgObject 的 ObjectId，以及 from == actor、to == [group_did]、kind == group_msg；
   以 JWT 形式提交时校验签名，且 kid 必须属于 from（MsgObject 规范 16.5），失败拒绝（bad-signature）
2. 解析目标 Session：to_session 缺省时为默认 Session；指定时该 Session 必须存在且 actor 是有效成员，否则 not-found；
   Session 已归档时拒绝（session-archived）；带 thread.topic 而不带 to_session 时拒绝（§2.5 过渡规则）
3. 校验消息关系与提及（§2.6）
4. 按 §4.1 判定 session.post（回应为 react），包括频率限制和 Hook
5. 幂等写入：在同一事务中写入 MsgObject、GROUP_INBOX 记录，并分配 group_seq / session_seq / accepted_at_ms
6. 为 host 本地的有效成员写投影记录（§5.4）
7. 为 tunnel 成员创建 DeliveryRecord（§5.4）
8. 返回 accepted（带 session_seq），发布变更通知
```

- 第 8 步的 accepted 表示群已经接受这条消息，并且 Hook 已经通过；不表示任何成员已读。
- **幂等**：同一个 `msg_id` 重复提交时，返回第一次的**终态**结果，即 accepted 或不可重试的 rejected，不会再次执行 Hook。可重试的失败（Hook 不可用、频率限制、host 暂时故障）不缓存，重试时重新判定。
- **Gateway 缓存**：公网 Gateway 返回 `cached` 时（Message Center §4.5），Hook 还没有执行，缓存转投到 host 后仍可能被拒绝。发送方必须以最终的 accepted / rejected 为准：按 CYFS dispatch 约定继续重试，或查询 `dispatch-status`。host 为每个 `(target, obj_id)` 保留一段时间的终态结果，供查询使用。
- 跨 Zone 写入使用 CYFS dispatch（§11.1）。

### 5.3 读取

读取以 pull 为准。读取结果是**针对该读者过滤后的视图**，不是 host 的全局视图。

**读取权的含义**：host 在读者获取消息时判定权限。读者已经获取的副本（本地投影、成员 Zone 的同步副本、tunnel 平台上的消息）不因后续撤权而回收。这与主流 IM 的做法一致；撤权只影响之后的获取。

- **Session 列表**：只返回读者是有效成员的 Session；群主和具有 `group.read_all` 能力的读者可以看到全部。Session Guest 只能看到自己作为 Guest 加入的 Session。读者无权访问的 Session，其存在、标题和成员数都不泄露。含 Guest 的 Session 对其参与者带 `has_guests` 标记。
- **Session 消息**：按 `session_seq` 分页。`history = FromJoin` 时只返回落在读者可见区间内的消息（§2.3.5）；`history = All` 时返回全部。绑定了 `read` Hook 的 Session，先按 Hook 判定读者能否读取该 Session（§7.3）。已撤回的消息只返回占位。
- **按 ObjectId 取消息**：只有当这条消息在读者可读的某个 Session 中有记录、并且对读者可见时才返回。已撤回的消息返回 `not-found`。知道 ObjectId 不等于有权读取。
- **附件**：见 §5.7。

### 5.4 成员侧投影与投递

| 成员位置 | 方式 | 说明 |
|---|---|---|
| 与 host 同 Zone | 写投影记录 | 为有效成员写 `INBOX` 记录：`owner = member_did`，带 `group:<group_did>` tag，`session_id` 为成员的本地 Session 键（§5.5），`sort_key = accepted_at_ms`，并记录 `session_seq`。不复制 MsgObject |
| 其它 Zone 的原生成员 | 成员 Zone 拉取变更流 | 成员 Zone 的 Message Center 作为 Joined Group 同步器，每个群维护一个变更流 token（§5.4.1），并在本地生成投影记录。host 发送不含内容的变更提示来加速拉取 |
| tunnel 成员（shadow DID） | 逐成员 DeliveryRecord | 外部平台无法拉取，所以由 host 推送；在创建 DeliveryRecord 时完成读取判定。关系消息（编辑、撤回、回应）也走 DeliveryRecord，由 tunnel 映射为平台操作，平台不支持时用文字说明 |

- 上表同样适用于 Session Guest：同 Zone 的 Guest 得到投影记录，其它 Zone 的原生 Guest 拉取，外部平台的 Guest 走 DeliveryRecord。客服场景中，经 Telegram 等平台咨询的客户就属于最后一种。
- 投影记录和 DeliveryRecord 只为写入时刻的有效成员生成。成员资格后来发生变化，不回改历史投递。撤回是例外，按 §2.6 尽力传播。
- 第一版不采用「写入时把 MsgObject 推送到原生成员 Zone」。由于读取权在获取时判定（§5.3），推送与拉取的权限语义相同，将来可以作为优化引入，推送前执行同样的判定（包括 `read` Hook）。
- 同一成员 Zone 内有多个本地用户加入同一群时，各自独立拉取，因为读取结果按读者过滤。

#### 5.4.1 变更流

每个读者对每个群有一条变更流，按 `group_seq` 顺序返回该读者可见的变化：

- **消息项**：Session 地址、`session_seq`、`obj_id`、是否已撤回。包括普通消息、关系消息和 Action Log 事件。
- **Session 项**：读者可见的 Session 新建、归档、删除；读者对某个 Session 的可见性变化（被加入、被移出）；SessionSharedState 的 revision 变化。
- **成员项**：读者自己的成员资格变化。

约束：

- token 对读者不透明。host 不能让读者从 token 推算出不可见 Session 的活动量，例如不能直接返回原始 `group_seq`。
- token 过旧、对应数据已经删除时，host 返回 `limited`。成员 Zone 随后按 Session 用 `session_seq` 重新拉取。
- 变更提示只包含 `group_did`，不包含 Session 和内容。host 对同一成员 Zone 的提示做合并，短时间内的多次变化只发一次。成员 Zone 也可以定时拉取作为兜底。

### 5.5 成员本地 Session 键

成员侧的本地 Session 键由成员自己的 Message Center 决定（Message Center §5.4），但必须满足：

- **按群区分。** 不同群里同名的 Session 不能合并成同一个本地 Session。当前实现直接用 topic 作为本地 session_id，不满足这一点。
- **关联权威状态。** 本地会话登记（`owner_sessions`）要记录 `SessionStateRef { authority_did: group_did, session_key }`，用于读取共享状态和 Action Log。

建议格式：本地 Session 键就是该 Group Session 的规范 MailboxAddress 字符串，即默认 Session 用 `<group_did>`，具名 Session 用 `<group_did>/<编码后的 session_id>`，与 `SessionStateRef.session_key` 相同（§2.2.4）。DID 中不含 `/`，在第一个 `/` 处拆分即可得到群和 Session，没有歧义。作为成员自己的 session_id 使用时，再按 MailboxAddress 规则编码为单个路径段。

### 5.6 回执

- 成员的已读状态是该成员自己的 `MailboxRecord.state`，不影响群记录，也不影响其他成员。
- 群内回执使用**已读水位**：host 保存 `(group_did, session_id, reader_did) → last_read_seq`，只前进不后退。本地成员由 Message Center 更新；远端成员由成员 Zone 经 §11.1 的 `read_markers` 上报，可以合并、延迟上报。
- 「某条消息有几人已读」由各读者的水位计算：水位不小于该消息的 `session_seq`，并且该消息在读者的可见区间内。
- 回执对其他成员的可见性由 Session 规则 `receipts` 决定：`Hidden`（缺省，只有本人看到自己的水位）、`Count`（只显示人数）、`Readers`（显示已读的人）。Session 有效成员数超过 `limits.receipts_max_members` 时按 `Hidden` 处理。
- 不再按 `(reader, msg_id)` 持久化逐条回执。

### 5.7 附件

- MsgObject 只携带附件引用（`content.refs` 中的 ObjectId 与 `uri_hint`），不携带内容（CYFS No-Push）。
- 获取附件是 CYFS 的多源下载。候选源包括：
  - 创作者，即发送者 Zone；
  - 收录者，即群 host；
  - 转发者，例如用户 A 把群 A 中的图片转发到群 B，群 B 的成员可以把 A 的 Zone 当作源。
- cyfs-ndn 的目标是提高大文件的分发速度。源的发现与内容校验分开：数据可以来自任何源，内容一律按 ObjectId / ChunkId 校验。
- **授权通过 context_path 完成。** 读者在请求链接中携带 context_path，说明自己经由哪条消息得到这个引用，例如 `<group_did>/<session_id>/<msg_id>`。各个源依据 context_path 自行决定是否返回数据。这属于 CYFS「带上下文的访问约束」，不是强访问控制。
- host 作为源时按 §5.3 判定：读者必须能读取 context_path 指向的消息，并且该消息确实引用了所请求的对象。知道 ObjectId 不等于有权获取。
- 转发产生新的 context。群 B 的读者使用指向群 B 中转发消息的 context_path，不需要能读群 A。
- host 是否主动拉取并保存附件、成为稳定的源，由 host 的存储策略和配额决定，不是协议要求。发送者删除内容或退群后，如果其它源都没有保存，附件可能取不到，这是 No-Push 的预期结果。
- context_path 的语法，以及非 host 源的校验方式，由 CYFS 协议定义（§14）。

---

## 6. 管理流程

### 6.1 创建群

1. 认证，并检查 `group.create` 权限（Zone RBAC）。
2. 生成或登记 Group DID，写入 DID Document（§2.1）。
3. 写入以下内容：
   - 配置；
   - Owner 的成员记录，以及经过真实签名的 proof；
   - 默认 Session，以及声明的具名 Session 记录。
4. 把 Group DID 登记为本 Zone 的本地收件方，使发往该群的 `post_send` 直接走本地 dispatch。
5. 发布 `entity.group_created` Action Log。

### 6.2 邀请与申请

- **邀请**：由具有 `group.invite_member` 能力的操作者发起。成员记录为 `Invited`，带有效期；并向被邀请者的个人 INBOX 发送邀请（`kind = operation`）。邀请到期变为 `Expired`，邀请者可以撤回（`Revoked`）。
- **邀请的接收**：被邀请者一侧按 Contact Mgr 的 ACL 处理邀请：陌生人发来的邀请进入 REQUEST_BOX，不直接打扰用户。Guest 邀请（§6.5.1）同样如此。
- **邀请链接**：具有 `group.invite_member` 能力的操作者可以创建邀请链接：

  ```rust
  pub struct GroupInviteLink {
      pub token: String,               // 不可猜测的随机值
      pub created_by: DID,
      pub expires_at_ms: Option<u64>,
      pub max_uses: Option<u32>,
      pub used: u32,
      pub require_approval: bool,      // true 时进入 PendingAdminApproval
      pub revoked: bool,
  }
  ```

  持有者携带 token 提交 proof（§11.1 `join?invite=`），得到 `Member` 角色。链接过期、用尽或被撤销后失效；创建者失去 `group.invite_member` 能力或离开群时，其创建的链接一并失效。`blocked` 的 DID 不能使用链接。
- **提交 proof**：只有被邀请者本人或其授权 agent 可以提交。校验通过后，如果 `join_policy` 要求审批，进入 `PendingAdminApproval`；否则直接进入 `Active`。
- **主动申请**：校验 proof 后进入 `PendingAdminApproval`，并向具有审批能力的成员的个人 INBOX 发送待审批通知。`join_policy = Open` 时直接进入 `Active`。
- **审批**：只能把 `PendingAdminApproval` 推进到 `Active`。不能复活 `Left` / `Removed` 的成员，也不能绕过 `blocked`。
- **拒绝**：状态变为 `Rejected`，并通知申请人。
- **Hook**：`join` Hook 在进入 `Active` 之前执行（§7.3）。
- 每次进入 `Active`，成员周期 `epoch` 加一（§2.3.3）。

### 6.3 退出、移除与删除群

- **退出群**（`group.leave`）：由成员本人发起，状态变为 `Left`，发布 `entity.member_left`。Owner 必须先转让 Owner 才能退群。
- **移除**（`group.remove_member`）：状态变为 `Removed`，发布 `entity.member_removed`，并向被移除者的个人 INBOX 发送通知。被移除者此后不能再读群消息，但应当收到这条个人通知。
- 以上两种情况，成员都同时失去全部 Session 的有效成员资格。已经投影到成员本地的历史不会被删除。
- **归档群**（`group.archive`）：所有 Session 不再接受新消息，历史仍按规则可读。
- **删除群**（`group.delete`）：只有 Owner 可以执行。host 在默认 Session 发布 `entity.group_deleted`，成员 Zone 经变更流得知后停止同步；host 随后删除群数据，为 `group_did` 保留墓碑，不再用它创建新群。

### 6.4 Session 管理

- **创建**：需要 `session.create` 能力。可以引用模板，指定成员列表声明和规则覆盖项；`session_id` 由 host 生成，或由创建者指定（不能与现存或已删除的 ID 相同）。`Explicit` Session 的创建者自动成为成员。`allow_guests = true` 时成员列表不能是 `Inherit`。
- **修改成员与规则**：需要 `session.manage` 能力，带 `expected_revision`。群成员部分必须是群的 Active 成员；群外 DID 只能按 §6.5 以 Session Guest 身份加入。
- **退出 Session**：
  - 群成员和 Guest 都可以退出具名 Session，系统写入一条 `Left` 记录；
  - 不能退出默认 Session，退出默认 Session 就等于退群。
- **个人偏好**：成员在本地设置的静音、置顶、归档只是个人偏好，不改变成员资格（Message Center §5.8）。
- **归档**：归档后的 Session 不再接受新消息，历史仍按规则可读。
- **删除**：需要 `session.manage` 能力。Session 记录变为 `Deleted` 墓碑，`session_id` 永不复用；host 删除该 Session 的 GROUP_INBOX 记录、成员表，以及只被它引用的 MsgObject。成员 Zone 已有的副本不受影响。默认 Session 不能删除。
- **Action Log**：每项变化都发布 Session 级 Action Log：`session.created`、`session.member_added`、`session.member_removed`、`session.member_left`、`session.rules_changed`、`session.archived`、`session.deleted`。Guest 的加入与退出使用同样的 action，载荷中标明参与者类型。这些 action 已列入 Session State and Action Log.md 的 §4.1。

### 6.5 Session Guest 的加入与退出

#### 6.5.1 由群内邀请

1. 具有 `session.invite_guest` 能力的操作者，向一个 `allow_guests = true` 的具名 Session 邀请 DID d。d 不能是被群封禁的 DID，Session 的 Guest 数不能超过上限（§6.7）。
2. host 写入一条 `Invited` 状态的 Guest 记录，并向 d 的个人 INBOX 发送邀请（`kind = operation`），邀请中只包含群的公开资料和该 Session 的标题。
3. d 提交作用域为 `Session(session_id)` 的 proof。外部平台用户以 tunnel 接入证据代替（§2.4）。
4. host 校验 proof 并执行该 Session 的 `join` Hook（如有），通过后记录变为 `Included`，发布 `session.member_added`。
5. 从此 d 是该 Session 的有效成员：按 §5.4 获得投影或投递，按 Session 规则读写。

应用也可以代表业务流程发起邀请。此时应用以群内具有相应能力的身份调用接口，客户仍需完成第 3 步的同意。

#### 6.5.2 由 Guest 发起请求

客服等场景通常由客户主动发起。群在配置中开启 `membership.guest_entry` 后，接受以下流程：

1. Guest d 选择一个随机的请求 ID，签署作用域为 `SessionRequest(request_id)` 的 proof，提交到 `guest_requests`（§11.1）。外部平台用户由 tunnel 代为提交，并附接入证据（例如用户在平台上主动给业务账号发了消息）。
2. host 检查：群开启了 `guest_entry`；d 没有被封禁；d 当前处于 Active 的请求 Session 数不超过 `max_open_per_guest`；频率限制。
3. 如果模板绑定了 `join` Hook，host 执行它，由应用决定是否受理。
4. host 用 `guest_entry.session_template` 新建一个 Session（`session_id` 由 host 生成），把 d 以 Guest 身份加入（`proof_id` 指向第 1 步的 proof），发布 `session.created` 与 `session.member_added`，并返回 Session 地址。
5. 应用可以随后用正常接口把具体客服加入 Session，或在模板中用 `Roles` 让某个角色的成员自动成为参与者。

同一个 `(d, request_id)` 重复提交返回同一个 Session。

**tunnel Guest 的入站路由。** 同一个外部平台用户可能通过同一个平台会话（例如与业务 bot 的私聊）参与同一群的多个 Session。tunnel 优先按平台的回复关系把入站消息映射到 Session；无法确定时，归入该 Guest 最近活跃的 Session。tunnel 在出站消息中应标明所属 Session（例如工单号），便于用户区分。

#### 6.5.3 退出与移除

- Guest 主动退出：记录变为 `Left`，发布 `session.member_left`。
- 被具有 `session.invite_guest` 能力的操作者移除，或被群封禁：记录变为 `Removed`，发布 `session.member_removed`，并向 Guest 的个人 INBOX 发送通知。
- Session 被归档或删除时，Guest 随之失去写入权或全部访问权，与群成员一致。

### 6.6 GroupEvent 与 Action Log

GroupEvent 是 host 内部的事件来源；对外的历史统一以 Action Log 消息发布（Session State and Action Log.md §5）。

- `event_id` 必须唯一。当前用「群 + 毫秒时间 + 类型」拼成 ID，同一毫秒内会碰撞，需要加入随机或序号成分。
- 状态提交与待发布日志在同一个事务中登记；发布失败后幂等重试。
- Action Log 事件与消息共用 `group_seq` / `session_seq`，因此入群、退群等提示与消息按正确的顺序交错显示。
- 日志的可见范围不超过对应 Session 的有效成员。

### 6.7 治理与反滥用

**上限。** 配置中的 `limits` 声明以下上限。群主只能调低，不能超过实现上限。建议默认值如下（待确认，§14）：

| 项 | 建议默认值 |
|---|---|
| 群成员数 | 500 |
| 处于 Active 的具名 Session 数 | 10,000 |
| 单个 Session 的 Guest 数 | 20 |
| 单条 MsgObject（canonical JSON） | 64 KiB |
| 回执可见的最大有效成员数（`receipts_max_members`） | 100 |
| 同一发送者在同一 Session 的发送频率 | 20 条 / 10 秒 |
| Agent 发送者 | 单独计数，默认为上一项的一半 |

超过群成员上限的大群需要单独设计大群模式（关闭回执、不展开成员列表等），第一版不做。

**治理工具。**

- 禁言、封禁（§2.4）；
- 删帖（§2.6）；
- 慢速模式（`SessionRules.slow_mode_ms`）；
- 频率限制（§4.1 第 7 步），Hook 不能放行；
- 带有效期和次数上限、可以撤销的邀请链接（§6.2）。

**骚扰与滥用。**

- 群邀请和 Guest 邀请发到被邀请者的个人 INBOX，接收侧按 Contact Mgr 的 ACL 处理，陌生人的邀请进入 REQUEST_BOX。
- `join_policy = Open` 的群容易被大量新 DID 涌入。Open 群建议配合 `join` Hook 或邀请链接使用；host 对入群请求同样限流。
- **Agent 参与者**：成员记录标明实体类型（§2.4），Message Hub 对 Agent 显示标识。Agent 运行时默认不响应其它 Agent 的消息，除非被提及，以免 Agent 之间互相触发形成消息风暴。

---

## 7. 扩展点

本节把扩展架构的 L1–L4 落到 Group Service 上。具体的 Hook 协议和面板嵌入协议另做专项设计，本节只确定接入位置和必须满足的约束。

### 7.1 L1：配置生成器

使用 §3.5 的一次性创建接口和 `group.apply_config`。生成器完成初始化后就可以退出，群的日常运行不会调用它。

### 7.2 L2：应用面板

```rust
pub struct PanelDecl {
    pub id: String,
    pub title: String,
    pub entry_url: String,   // 面板的 HTML 入口
    pub scope: PanelScope,   // Group | Sessions(Vec<String>)
}
```

- 面板在 `extensions.panels` 中声明，Session 通过 `SessionRules.panels` 引用。
- Message Hub 加载面板时传入 `group_did` 和 `session_id`。这些上下文只用于定位，不是授权。面板读写群数据时，要以当前用户（或经授权的应用）身份调用 Group Service，并按 §4.1 判定。
- **可信来源以群为准**：`entry_url` 的 origin 必须属于 Group DID Document 声明的 host，即群主的 OOD。判断基准不是读者自己的 Message Hub 所在的 Zone。
- 来源不符的面板默认不加载。用户显式接受后才可以加载，接受时要说明面板由谁提供、会拿到哪些群上下文。
- 面板加载失败只影响面板本身，不影响聊天。

### 7.3 L3：Hook

Hook 绑定在 Session 规则上。第一版只开放三个 Hook 点：

| Hook 点 | 执行时机 | 可以做的事 |
|---|---|---|
| `join` | 成员进入 `Active` 之前；Guest 进入 `Included` 之前；Guest 请求新建 Session 之前 | 拒绝；或在原生规则允许的前提下放行 |
| `post` | 消息（包括关系消息）写入 GROUP_INBOX 之前 | 拒绝；或对原生无发言权的成员给出动态许可（需要配置允许） |
| `read` | 读者开始读取或同步某个 Session 时。按 Session 判定，不逐条过滤消息 | 拒绝；不能扩大原生可见范围 |

```rust
pub struct HookBinding {
    pub point: HookPoint,               // Join | Post | Read
    pub service: String,                // Application Service 入口
    pub may_grant: bool,                // 是否允许给出动态许可
    pub on_unavailable: FailurePolicy,  // Deny | NativeOnly
    pub timeout_ms: u32,
    pub result_ttl_ms: u32,             // 仅 read：允许结果对同一读者、同一 Session 的缓存时长，0 为不缓存
}
```

**结果与故障策略：**

- Hook 返回三种结果之一：允许、拒绝、不可用（超时或服务错误）。
- **拒绝永远是拒绝**，不能被故障策略放行。
- `on_unavailable = NativeOnly` 表示按原生权限继续：原生允许的照常允许，动态许可失效。`Deny` 表示直接拒绝。不提供「不可用时全部放行」的选项。
- `read` Hook 不可用时只能收窄可见范围，不能扩大。
- `read` 不逐条过滤消息，原因有二：读者已经获取的副本本来就无法回收（§5.3）；逐条过滤会让分页结果变短，并且每页都要调用 Hook。

**调用约束：**

- 同一个 `msg_id` 的 `post` Hook 的允许或拒绝结论随幂等结果一起保存，重试时不会重新调用；不可用不保存（§5.2）。
- Hook 以 Group Service 的身份调用 Application Service，只传判定所需的最小上下文：操作、actor、group_did、session_id，`post` 时再加上 MsgObject。
- Hook 位于频率限制之后（§4.1），被限流的请求不会调用 Hook。

**可见性：** 绑定 Hook 意味着群的关键操作依赖应用服务，并且 `post` Hook 能看到消息内容。这是应用开发者和群主的显式选择（扩展架构 §6.4）。Message Hub 应当在群信息中展示该群绑定了哪些 Hook。

### 7.4 L4A：自定义 UI

自定义应用直接调用 Group Service 和 Message Center 的公开接口，可以复用 Message Hub 的组件源码（源码级复用，不承诺兼容）。需要限制入口时，使用 §4.3 的访问路径限制。

### 7.5 L4B：独立实现

独立实现只需要满足 §11.1 的协议能力，它不继承默认实现的配置、Hook、备份和多副本能力。Message Hub 对第三方群只依赖 §11.1 的能力；客户端不能假设 DID Document 中的 endpoint 背后一定是 BuckyOS 的 Group Service。

---

## 8. Hosted Group 与 Joined Group

**Hosted Group**：本 Zone 是 host。本 Zone 的 Group Service 是权威，管理界面可以管理成员、Session 和配置。

**Joined Group**：本 Zone 的用户是成员，群由别处托管。本 Zone 的 Message Center 为每个本地成员保存：

- 群 DID 和显示资料缓存；
- 用户在该群可见的 Session 列表缓存，以及本地 Session 登记；
- 该群的变更流 token，以及各 Session 已同步到的 `session_seq`；
- 用户自己的阅读状态、已上报的已读水位和个人偏好。

它不保存成员资格的权威数据，也不能修改群状态，所有写操作都发往 host。

如果本 Zone 用户只是某些 Session 的 Guest，Joined Group 记录只包含这些 Session，不缓存群的其它信息。

- **向 Joined Group 发言**：构造群消息（用 `to_session` 指定 Session）并调用 `post_send`。原生投递按 Group DID 解析到 host 的 inbox 路径，并携带 `cyfs-original-user` 和成员证明。
- **外部平台群**：从外部平台同步进来的群（例如 Telegram 群）是 tunnel 连接，不是 Self-host Group，不使用本文的成员与 Session 模型。

---

## 9. 可靠性与备份

### 9.1 群核心数据

群核心数据由默认 Group Service 负责备份与恢复，范围包括：

- Group Configuration，以及各 revision 中必要的历史；
- 成员记录（含成员周期）、proof、moderation 标记和邀请链接；
- Session 记录与墓碑、Session 成员表、可见区间、SessionSharedState / SessionMemberState；
- GROUP_INBOX 记录（含序号）与 MsgObject（附件只备份引用，附件对象按对象存储的策略备份）；
- 序号计数器、已读水位、GroupEvent 和待发布的 Action Log；
- DID Document 及其 controller 信息。

撤回删除的 MsgObject 正文也要从之后的备份中删除。应用数据（例如积分、账号映射）由应用自己负责。恢复群之后，需要检查配置中引用的面板和 Hook 服务是否仍然可用，并提示群主。

普通成员的本地记录只是该成员的可见视图，不能当作群的备份。

### 9.2 多副本方向

后续允许由少量 Group Service 实例共同承载同一个 Group（主从结构），用于备份和故障切换。群主保留控制权，不引入成员间共识。复制、切换，以及如何在 DID Document 中表达副本信息，另行设计。序号只由主实例分配。

### 9.3 可验证归档

需要对外证明完整历史时，可以生成强一致的归档容器对象（即 v1 §5 中的 archive）。在线读取不依赖它。第一版不实现。

---

## 10. 安全与一致性要求

1. 操作者只取自认证上下文，不信任请求中的 DID 字段。
2. 群消息的 `from` 必须等于认证得到的 actor，`to` 必须恰好是群 DID，目标 Session 只由 `to_session` 决定。
3. 写入、读取、管理全部在 host 上判定。成员 Zone 不能绕过 host 写入其他成员的 INBOX。
4. 成为群成员或 Session Guest，都必须有经过签名校验的 proof（外部平台用户以 tunnel 接入证据代替），且 proof 只能由本人或其授权方提交。
5. 角色只能通过邀请或授权变更获得，不能在申请中自报。
6. Session 是隔离单元：读者无权访问的 Session，其存在、内容和成员都不泄露；对非有效成员，「不存在」与「无权访问」返回同一个 `not-found`；按 ObjectId 读取同样受限。
7. Session Guest 只能访问自己被加入的 Session，看不到默认 Session、其它 Session 和群成员列表。
8. 发往未知 Session 的消息一律拒绝，不落入默认 Session。
9. Hook 的拒绝不能被故障策略放行；Hook 不可用时不扩大可见范围；Hook 不能放行被频率限制拒绝的请求。
10. 面板的加载许可不等于数据访问授权；面板来源以群 host 为准。
11. `display` 配置不是授权。
12. 历史投递按写入时的有效成员决定。成员变化不回改历史，也不删除成员已经获得的历史。撤回和删帖是例外：host 删除正文，并尽力传播到成员副本（§2.6）。
13. 跨 Zone 写入只投递小型 NamedObject，附件只传引用（CYFS No-Push）；获取附件携带 context_path，host 作为源时按 §5.3 判定。
14. 所有对外对象（MsgObject、proof、GroupDoc、Action Log 消息）都能通过 canonical JSON 重算 ObjectId。以 JWT 形式提交的 MsgObject，host 必须校验签名，并保存、原样提供 JWT 原文。
15. 删除群、退出群、退出 Session、删除本地会话历史是四种不同的操作。
16. 配置、成员和 Session 的变化都有 Action Log；至少群主可以查看完整审计记录。
17. 排序、同步游标、可见区间和回执只使用 host 分配的序号，不使用 `created_at_ms`。
18. `session_id` 和 `group_did` 删除后都不复用。

---

## 11. 接口

### 11.1 协议能力（L4B 互操作边界）

独立实现与 BuckyOS 客户端之间只依赖以下内容：

```text
DID Document     实体类型 group、host、controller、服务路径前缀

写入
PUT  cyfs://$host/<group_did>/inbox                          写入默认 Session（MsgObject 不带 to_session）
PUT  cyfs://$host/<group_did>/sessions/<sid>/inbox           写入具名 Session（<sid> 必须等于 MsgObject.to_session）
PUT  cyfs://$host/<group_did>/join[?invite=<token>]          提交入群申请或 proof；可附带邀请链接 token
PUT  cyfs://$host/<group_did>/sessions/<sid>/join            Session Guest 提交 proof
PUT  cyfs://$host/<group_did>/guest_requests                 Guest 发起请求，host 为其新建 Session
PUT  cyfs://$host/<group_did>/read_markers                   上报已读水位

读取
GET  cyfs://$host/<group_did>/changes?since=&limit=          读者可见的变更流（§5.4.1）
GET  cyfs://$host/<group_did>/sessions                       读者可见的 Session 列表
GET  cyfs://$host/<group_did>/inbox?after_seq=&limit=
GET  cyfs://$host/<group_did>/sessions/<sid>/inbox?after_seq=&limit=
GET  cyfs://$host/<group_did>/objects/<obj_id>?context_path=  受群 ACL 约束的对象读取（§5.7）

对象   MsgObject（§2.5 的群形态，含 §2.6 的关系消息）、GroupMemberProof、Action Log event
```

- 消息、proof 的 `PUT` body 是 canonical JSON NamedObject；签名的消息用 JWT 形式（`application/cyfs-named-object+jwt`，见 CYFS dispatch）。路径不带 `/@/`，body 不带附件内容。`objects/<obj_id>` 对有 JWT 原文的消息返回 JWT 形式。`read_markers` 的 body 是小型 JSON 对象 `{ session, last_read_seq }`，读者取自认证上下文。
- 所有请求都携带 `cyfs-original-user` 和 `cyfs-proofs`。`GET` 返回针对该读者过滤后的结果；对非有效成员按 §4.1 返回 `not-found`。
- `inbox` 列表按 `session_seq` 返回 `{ items: [{ seq, obj_id, redacted }], next_after_seq, limited }`，每页最多 4096 项。
- `<sid>` 按 MailboxAddress 规则编码为单个路径段。

> 待确认：Session 列表、变更流和受控对象读取的路径形态，以及 context_path 的语法（§14）。

### 11.2 默认实现的 RPC

以下是 BuckyOS Group Service 的 kRPC 接口（`/kapi/msg-center`），属于默认实现，不属于协议。所有请求都不带 `actor_did` 和 `host_owner`：操作者来自 token，群由全局唯一的 `group_did` 定位。

| 分类 | 方法 |
|---|---|
| 群 | `group.create`、`group.get_doc`、`group.get_config`、`group.apply_config`、`group.archive`、`group.delete`、`group.transfer_owner` |
| 成员 | `group.invite_member`、`group.revoke_invite`、`group.create_invite_link`、`group.revoke_invite_link`、`group.submit_member_proof`、`group.request_join`、`group.approve_member`、`group.reject_member`、`group.leave`、`group.remove_member`、`group.update_member_role`、`group.moderate`、`group.list_members` |
| Session | `group.create_session`、`group.update_session`、`group.list_sessions`、`group.invite_session_guest`、`group.submit_session_proof`、`group.submit_guest_request`、`group.remove_session_member`、`group.leave_session`、`group.archive_session`、`group.delete_session` |
| 查询 | `group.list_by_member`、`group.check_access(group_did, action, session_id?)`、`group.list_events`、`group.update_read_marker` |

- **读取消息**：成员通过 Message Center 已有的 Session API 读取自己的投影。群主或具有 `group.read_all` 能力的管理者用 `list_box_by_time(mailbox = group_did[/sid], GROUP_INBOX)` 读取，同样经过 §4.1 授权。
- **发送消息**：使用已有的 `post_send` / `dispatch`，不再新增 `post_group_send` 一类接口。编辑、撤回、回应也是发送一条带 `relates_to` 的消息，不新增 RPC。

---

## 12. 实现现状与迁移

当前状态：v1 的 GroupMgr、共享群类型、六张群表的建表定义、`group.*` RPC，以及前端旧群数据接入已删除，v2 尚未实现。Message Center 的通用消息存储、MailboxAddress、外部群订阅投影、Session API 和 tunnel 链路继续保留。删除不包含对已部署数据库的清表或迁移。

以下 §12.1–§12.4 保留为删除前的审查记录，供 v2 实现参考，其中旧群代码、接口和表的描述不再代表当前实现。

本节依据 2026-09-30 对以下代码的审查：`src/frame/msg_center/src/group_mgr.rs`、`msg_center.rs`、`cyfs_dispatch.rs`、`src/kernel/buckyos-api/src/group_mgr.rs`、`cyfs-ndn/src/ndn-lib/src/msgobj.rs`，以及 Desktop 前端。

### 12.1 与 v2 一致、可以直接沿用的部分

- 群消息使用 `from=actor, to=[group]`，没有 `source` 字段，MsgObject 不被改写。
- `MailboxAddress`（`$did/session_id`）、按精确 inbox 授权、`list_mailboxes` 已经实现。
- 群消息写入时生成一条 GROUP_INBOX 记录，并为成员生成带 `group:<did>` tag 的 INBOX 投影记录。
- 成员记录、proof 存储、GroupEvent 存储，以及邀请和审批 RPC 已有基本的 CRUD。
- 前端发送群消息时设置 `to_session = session.id`，后端 `derive_session_id` 优先取 `to_session`，因此事实上已经支持群内多个 Session（2026-09-30 已从 `thread.topic` 迁移）。

### 12.2 必须优先修复的安全问题

| 问题 | 位置 |
|---|---|
| 群消息写入没有发言权限检查，只检查发送者是否被拉黑。任何人都能写入任意托管群，AdminOnly 形同虚设 | `msg_center.rs` dispatch 的群分支；`cyfs_dispatch.rs` 只校验 `from == principal` |
| group RPC 信任请求中的 `actor_did` / `host_owner`，handler 不使用 RPC 上下文 | `msg_center.rs` 的 `handle_group_*`；`kernel/buckyos-api/src/group_mgr.rs` 的请求类型 |
| proof 只检查过期时间和 scope，不做签名校验，也不检查是谁提交的 | `group_mgr.rs` 的 `validate_member_proof`、`submit_member_proof` |
| 主动申请直接采用 proof 中的 role，可以申请到 Owner/Admin | `group_mgr.rs` 的冷申请分支 |
| `approve_member` 不检查当前状态，可以复活已移除的成员 | `group_mgr.rs` 的 `approve_member` |
| RBAC 给 `users` 组授予了 `obj://msg-center/group_inbox/*` 的读写权限 | `kernel/buckyos-api/src/rbac_config.rs` |

### 12.3 功能缺口与迁移项

| v2 要求 | 现状 | 迁移 |
|---|---|---|
| MsgObject 新字段（`to_session`、`relates_to`、`mentions`），删除 `proof` 字段 | 已完成（2026-09-30）：ndn-lib 与 Desktop TS 镜像已同步；入口用 `MsgObject::validate()` 校验 | — |
| 签名消息以 JWT 形式投递和保存 | 已完成接收与保存（2026-09-30）：ndn-lib、Gateway 缓存与 msg-center 接受 `application/cyfs-named-object+jwt`，msg-center 验签（kid 须为 `from` 自己的密钥）后把 JWT 原文存入 `msg_jwt_originals`，原生转投时发送 JWT | 提供 JWT 原文的读取接口；支持 `from` 授权的设备 / Agent 密钥 |
| 目标 Session 由 `to_session` 指定 | 已完成（2026-09-30）：前端、OpenDAN、Telegram tunnel 改用 `to_session`，`derive_session_id` 不再取 topic；只带 topic 的群消息以 `missing-to-session` 拒绝 | 校验 Session 存在（随 Session 记录实现） |
| host 分配序号 | GROUP_INBOX 记录的 `sort_key = created_at_ms`，没有 `group_seq` / `session_seq` | 新增序号字段与计数器，`sort_key` 改为 `accepted_at_ms` |
| Group Session 作为一等对象，带成员列表和规则 | 只有由 topic 派生的 session_id，没有 Session 记录、成员表和规则 | 新增 Session 记录表、成员表、模板与 RPC；删除 subgroup（`group.create_subgroup` 等） |
| 默认 Session 使用裸 `group_did` | 没有 topic 的群消息，`session_id` 被设为 group_did 字符串 | 修改 `derive_session_id` 的群分支（待确认，§14） |
| 发往未知 Session 的消息被拒绝 | 任意 topic 都会成为新的 session_id | 写入时校验 Session 存在、发送者是有效成员 |
| 群消息不能 `move_record` | `move_record` 对群记录同样可用 | 对 GROUP_INBOX 记录禁用 |
| 成员侧本地 Session 键按群区分 | 成员 INBOX 直接用 topic 作 session_id，不同群的同名 topic 会被合并 | 按 §5.5 生成 |
| 读取按读者过滤，可见区间 | 普通用户读不到 GROUP_INBOX，只能读写入时生成的投影；服务和 Agent 可以读取全部；没有成员周期与可见区间 | 按 §2.3.5、§5.3 实现 |
| 变更流 | 没有 | 按 §5.4.1 实现 |
| 消息关系与提及 | 没有 | 按 §2.6 实现，并在 tunnel 中映射为平台操作 |
| Group Configuration | GroupSettings 是封闭枚举，建群后无法修改 join/post policy；GroupDoc 与 settings 内容重复 | 按 §3 重建，GroupDoc 改为派生 |
| Group DID 与 DID Document | DID 形如 `did:bns:group-<owner>-<host>-<name>-<ms>`，不写 DID Document | 按 §2.1 |
| hosted group 登记为本地收件方 | 未登记。`register_local_recipients` 只在启动时为 tunnel 绑定调用，向群 `post_send` 会得到 `native-route-not-configured` | 建群时登记 |
| dispatch 时的群作用域 | `active_singleton_members` 用调用方的 owner_key 加载群，展开时却传 `host_owner: None`，导致用户作用域的群报 not found | 去掉 host_owner 作用域后，这个问题自然消失 |
| 远端成员 | 只在 host 本地库写 INBOX 记录，没有跨 Zone 投递 | 实现 Joined Group 同步（§8） |
| 回执 | 只存在内存中 | 按 §5.6 实现已读水位 |
| 个人 INBOX 通知（邀请、待审批、移除） | 没有 | 按 §6.2–§6.3 实现 |
| 邀请链接、Guest 请求、频率限制、慢速模式 | 没有 | 按 §6.2、§6.5.2、§6.7 实现 |
| `group.leave`、`transfer_owner`、`archive` / `delete`、`apply_config` | 没有 | 按 §11.2 实现 |
| Session Guest | 没有。现有 `GroupRole::Guest` 是群角色，Guest 必须先入群 | 新增 Session 成员表与 Guest 流程（§2.3.2、§6.5） |
| GroupEvent 发布为 Action Log | 只写入 `group_events` 表；event_id 在同一毫秒内会碰撞 | 按 §6.6 实现 |
| CYFS 群路径（变更流、join、sessions、guest_requests、read_markers） | 未实现，只有静态配置的 inbox 路径 | 按 §11.1 实现 |
| Hook、面板、访问路径 | 没有 | 按 §7 实现，实施顺序遵循扩展架构 §11.3 |
| 前端 | 只调用 `group.list_by_member` 和 `group.check_access`；`self_host_groups.ts` 是 mock，类型与后端不一致；Users & Agents 中的「Open in MessageHub」按钮没有响应 | 随 RPC 落地逐步接入 |

### 12.4 需要删除的 v1 实现

以下 v1 实现需要删除：

- 嵌套群展开：`group.expand_members`、`group.list_parents`、`GroupExpansionSnapshot`；
- 收益归属：`group.update_attribution_policy`；
- `GroupCollectionPolicy`、`GroupPurpose`；
- `GroupRole::Guest`，以及 proof 中的 `JoinAsSelf` / `JoinAsCollectionEntity` 作用域（改为 §2.4 的 `MemberProofScope`）；
- subgroup 相关的类型与 RPC。

beta 2.2 不需要兼容，直接删除即可。删除后，dispatch 每次写入一条展开快照的行为也随之消失。

---

## 13. 迁出与后置

- **DID Collection（嵌套群、递归展开、协作署名与收益归属）**：迁出到独立的 DID Collection 设计，v1 的 §1.1、§2.1、§2.9、§4.7、§6.10、§6.11 作为该设计的输入。在此之前，`member_did` 只接受单体 DID，但字段类型仍保持为通用 DID。
- **公开成员的双向证明**：proof 照常保存，对外公开成员列表和反向证明查询后置。
- **多副本**：见 §9.2。
- **可验证归档**：见 §9.3。
- **大群模式**：超过 `limits` 群成员上限的群，见 §6.7。
- **一级 Group DID**：BNS 合约创建、续费和 controller 迁移的 UI 后置。
- **外部平台群迁移为 Self-host Group**：后置。

---

## 14. 待确认事项

| # | 问题 | 本文建议 |
|---|---|---|
| 1 | 默认 Session 使用裸 `group_did`（session_id 为 NULL），还是沿用当前以 group_did 字符串作为 session_id 的做法 | 使用裸 `group_did`，与「裸 DID 是默认 inbox」一致 |
| 2 | 成员侧本地 Session 键与 `SessionStateRef.session_key` 的格式 | 都用 Group Session 的规范 MailboxAddress 字符串：`<group_did>` / `<group_did>/<sid>`（§2.2.4、§5.5） |
| 3 | 跨 Zone 访问时，如何携带并验证客户端（应用）身份 | 与 verify-hub、CYFS proofs 一起设计 |
| 4 | Session 列表、变更流和受控对象读取的 CYFS 路径形态 | 见 §11.1 草案 |
| 5 | 面板上下文是否包含 viewer 身份和短期凭据 | 不包含，面板自行以用户身份认证 |
| 6 | host 向成员 Zone 发送「变更提示」的对象格式与路径 | 小型 event NamedObject，只含 `group_did`，不含 Session 与消息内容，host 合并后发送 |
| 7 | 外部平台用户（shadow endpoint DID）无法签名，入群或成为 Guest 时以什么作为同意证据 | 由 tunnel 实例出具接入证据（`attested_by = transport_did` + 平台来源事件），见 §2.4 |
| 8 | Guest 能看到的 Session 参与者范围 | 显式参与者，加上在该 Session 发过言的参与者；不展开 `Roles` 带来的全部群成员（§3.3） |
| 9 | context_path 的语法，以及创作者、转发者等非 host 源如何校验 | 由 CYFS 协议定义；host 作为源时按 §5.3 判定（§5.7） |
| 10 | 规模与频率上限的默认值 | 见 §6.7 表 |
| 11 | 变更流 token 的编码 | 不透明；不能暴露原始 `group_seq` |

已确认：

- Guest 不入群，以 Session Guest 身份加入具体 Session，Session 成员列表可以是群成员的超集（§2.3.2）。
- 信任模型：self-host 即全听群主的，消息默认由群主背书；需要强证明时用 MsgObject 的 JWT 签名形式，这是协议级能力（§1.5）。
- 附件通过 context_path 授权，按创作者、收录者、转发者多源获取（§5.7）。
- MsgObject 做 breaking 修改：增加 `to_session`、`relates_to`、`mentions`；删除 `proof` 字段，签名统一用标准对象的 JWT 形式；`thread.topic` 只作语义 hint（MsgObject 规范）。
- 2026-09-30 评审采纳项：host 分配序号；每群一条变更流；Session 移出配置；读取权在获取时判定、`read` Hook 改为 Session 级；对非成员统一返回 `not-found`；ID 不复用与成员周期；已读水位；治理与反滥用工具；Guest 的外部成员标识与主动请求流程。

---

## 15. 文档联动

- **cyfs-ndn《CYFS 标准对象》§16**：MsgObject v2 定义（`to_session`、`relates_to`、`mentions`，删除 `proof`，签名用 JWT 形式）。已随本次修订更新。
- **cyfs-ndn《CYFS Protocol》dispatch**：body 可以是签名 JWT（`application/cyfs-named-object+jwt`）。已随本次修订更新。
- **Message Center.md**：§2.1 MsgObject 字段说明、§2.3 `sort_key` 规则、§2.3.1 发往具名 Session 的方式已随本次修订更新。§2.3.1 中「群消息的 session 推导为群 DID」需要随 §14#1 的结论更新；§3.2 补充群消息按 Session 写入；§7 中关于回执的引用改为指向本文。
- **Session State and Action Log.md**：Group Session 的 `SessionStateRef` 约定与 §6.4 列出的 Session 级 action 已随本次修订补充。
- **CYFS Protocol.md**：context_path 的语法与多源校验（§14#9）。
- **Contact Mgr.md**：说明 `Contact.groups` 只是联系人分组，与群无关；群邀请按 ACL 进入 INBOX 或 REQUEST_BOX。
- **UI_DATAMODEL.md**：群的 Session 列表改为来自 `group.list_sessions`，去掉 subgroup 作为来源；增加「含外部成员」「群主可查看」标识、撤回占位与回应展示。
- **Self-Host-Group.md（v1）**：在文首标注「已被 v2 取代」。
- **扩展架构**：本文 §7 是扩展架构 L1–L4 在 Group Service 上的具体落点。
