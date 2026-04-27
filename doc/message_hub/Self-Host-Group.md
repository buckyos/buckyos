# Self-Host Group 组件需求

## 1. 背景与目标

本文基于 `Message Center.md` 和 `Contact Mgr.md` 的当前设计，说明 BuckyOS 在现有 MessageHub 框架下支持 **self-host group** 需要补齐的组件能力。

更底层地，本文把 group 放到 **DID 作为下一代互联网实体标识** 的框架下讨论。在 BuckyOS 的基础协议里，Zone、Owner/People、Device、Agent 都已经是有明确语义的实体：它们有 DID、有 DID Document、有可验证控制权，可以作为权限判断主体，也可以作为消息投递或服务调用的目标。

这里的 group 指 **MessageHub 中可收发消息的集合型实体**，不是 ContactMgr 里的普通联系人标签、权限集合或 UI 分类。判断标准是：这个 group 是否拥有自己的 DID，是否可以像 Owner、Device、Agent 一样作为 `MsgObject` 的会话对象，是否可以在 MessageCenter 中拥有 `GROUP_INBOX`、成员订阅和群消息历史。

Self-host group 的含义是：

1. group 由当前用户的 Zone/OOD 托管，当前 Zone 是该 group 状态的真相源。
2. group 拥有独立 `Group DID`，在 Users & Agents / MessageHub 中是一级可管理实体，类似 agent。
3. group 的成员、角色、消息历史、投递策略和 read receipt 由 host Zone 维护。
4. 第一版不做多 host 共识，也不做多个 OOD/Zone 共同托管同一个 group 的协议。

### 1.1 下一代互联网中的 DID 实体模型

在 BuckyOS 的基础协议里，DID 不只是登录账号或联系人 ID，而是实体在网络中的稳定名字。一个可被系统管理的 DID Entity 至少具备以下能力：

1. **可寻址**：其它实体可以解析 DID Document，找到该实体的服务入口、host Zone/OOD 或路由信息。
2. **可授权**：DID Document 定义 controller、key、授权 agent 和根权限，应用层角色不能覆盖 DID Document 的控制权。
3. **可投递**：实体可以作为消息、通知、事件或任务的投递对象。
4. **可验证**：实体声明、成员关系、授权关系和重要状态变化都应能被签名或 proof 验证。
5. **可组合**：实体可以被其它实体引用，用于权限范围、协作关系、内容归属、组织关系或 agent 推理。

当前系统里已经有多种 **单体实体**，例如 Zone、Owner/People、Device、Agent。它们的 DID 通常指向一个相对单一的主体或执行单元。

Self-host group 引入的是 **集合型实体**：一个 DID 指向一组 DID，并且这个集合本身仍然是一个完整实体。调用方在“作者、收件人、权限主体、收益归属主体、协作主体”等位置看到一个 DID 时，原则上不应该预设它一定是个人、设备或 agent；它也可以是一个集合。只有当业务需要展开成员、验证成员关系或执行投递计划时，才需要识别它是否为 DID 集合。

本文明确：BuckyOS 第一版只定义一种集合型实体(我们也希望这是唯一的一种)，即 self-host group。集合内部的成员仍然是 DID，因此成员可以是单体实体，也可以是另一个 group DID。这使 group 成为递归的 DID 集合定义，而不是在联系人分组、权限集合、群聊、组织、团队之间发明多套不兼容的集合模型。

### 1.2 设计目标

1. **抽象 DID 集合型实体**：把 group 定义为拥有独立 DID、DID Document、成员证明和 MessageHub 入口的集合型实体，而不只是聊天群。
2. **复用现有消息模型**：继续使用不可变 `MsgObject`、可变 `MsgRecord`、`GROUP_INBOX`、per-reader `MsgReceiptObj`。
3. **复用 ContactMgr 的身份与路由能力**：group 成员仍是 DID；外部账号仍通过 `AccountBinding` 和 tunnel 路由。
4. **明确 self-host 权责**：host Zone 负责成员管理、写入校验、群 inbox 目录、跨 Zone 小对象 `dispatch` 入口和群信息展示。
5. **以 DID/DID Document 为权限根**：group 的身份、控制权、公开入口和关键权限都应能从 `Group DID` 及其 DID Document 推导。
6. **成员资格双向确认**：成员进入 `Active` 前必须有来自成员 DID 的有效签名，证明该用户愿意成为 group member；当成员本身也是 group DID 时，签名应来自该 group DID 的 controller 或授权 agent。
7. **支持递归集合语义**：GroupMemberRecord 的 `member_did` 可以引用另一个 group DID，系统必须能以有界、可审计、可防循环的方式处理嵌套集合。
8. **保持第一版简单**：单 host 权威模型，远端成员只是参与者；加入的外部 group 只作为 Contact/Joined Group 管理，不承担托管职责。

### 1.3 非目标

1. 不设计去中心化群共识协议。（未来通过集合成员的hash和消息记录的hash可以升级成共识协议)
2. 不要求群消息在所有成员 Zone 上形成一致的可写副本。
3. 不把普通联系人分组升级为 messageable group。
4. 不在本文定义收益结算、版权登记或链上分账协议；本文只定义 group DID 能作为协作署名和收益归属的身份锚点。
5. 不在本文定义完整 UI 交互细节，只列出 UI 需要读取或触发的后端能力。

### 1.4 与 CYFS Protocol 的对齐原则

本文不修改 CYFS Protocol 的身份、传输、支付或内容发布边界。CYFS 协议层只消费 W3C DID、定义 `NamedObject` / `NamedData` 的可验证获取、`cyfs://` 语义路径、跨 Zone 小对象 `dispatch` 和 Pull-first 的内容传播约束；self-host group 是 BuckyOS / MessageHub 在这些能力之上的应用层实体模型。

后续所有 app 如果要把某个 DID 实体做成可协作、可投递、可审计的集合，都应遵循下面的 CYFS 对齐规则：

1. **对外对象必须是 `NamedObject`**：`MsgObject`、`GroupDoc`、`GroupMemberProof`、公开成员证明、`GroupEvent`、归档快照等需要跨 Zone 传输或长期验证的对象，都应使用 CYFS canonical JSON 形成稳定 `ObjectId`。可选字段缺省时应省略，不能用 `null` 代替缺省。
2. **跨 Zone 写入只能是小对象 `dispatch`**：向 group 发消息、提交入群 proof、申请加入、投递管理通知，本质都是 `PUT cyfs://$group_ood/<sem_path>` 到 host Zone 控制的逻辑接收点。body 是 canonical JSON `NamedObject`，不能直接携带 Chunk、附件或大文件内容。
3. **附件和大对象只传播引用**：消息中的图片、文件、长文档、索引目录等只能以 `ObjectId`、`FileObject`、`ChunkList` 或语义 URL 形式引用。host Zone 接受消息后不因 `dispatch` 自动下载附件；reader 或 host 根据 policy 后续主动 Pull，并验证 `PathObject`、`NamedObject`、`ChunkId`。
4. **inbox 是语义路径目录**：`/<group_did>/inbox`、`/<group_did>/sub/<subgroup_id>/inbox` 这类高写入路径默认按 CYFS 形态 B 处理，即 `list=loose` 的尽力而为目录。列表本身是 host 当前视图，返回 child `ObjectId`；每个 child 对象仍可独立校验。
5. **封版历史才使用强一致容器**：如果需要发布可多源验证的群归档、成员快照或审计包，应生成独立 container `ObjectId`，再用 `PathObject` 把语义路径绑定到该对象。在线 inbox 不应为了每条消息重算一个强一致目录根。
6. **ACL 属于 host service**：CYFS `dispatch` 只定义请求语法，不定义群成员权限。host Zone 的 GroupMgr/MessageCenter 应结合 `cyfs-original-user`、`cyfs-proofs`、成员表、DID Document controller 和 group policy 做写入判定。
7. **语义路径不使用 `inner_path` 写入**：`dispatch` 目标只能是 `cyfs://$zoneid/<sem_path>`，不能带 `/@/`。如果要更新对象内部字段，应投递新的不可变对象，由 host service 决定如何落地到当前状态。

---

## 2. 核心概念

### 2.1 DID 实体与 DID 集合

BuckyOS / MessageHub 的应用协议只承认一种主体：**DID 实体**。CYFS Protocol 本身不定义身份系统，只消费 DID；Zone、Owner（People）、Device、Agent 是 BuckyOS 当前已经存在的单体实体类型；Group 是即将引入的第一个集合型实体。无论单体还是集合，对外都以 DID 形式出现，调用方在大多数场景下不需要、也不应该区分一个 DID 指向的是个人、设备、agent，还是一个集合。

#### 2.1.1 DID 实体的基础能力

任何 BuckyOS DID 实体都必须具备以下能力。这是协议层对所有实体类型的统一承诺：

1. **可识别**：拥有 DID，DID Document 公开可解析。DID 可以是二级形式（如 `xxx.zone_id`），也可以是一级 DID。
2. **可寻址投递**：可以作为 `MsgObject` 的 `from` / `to` / `source`。任意一方向该 DID 收发消息时，从 DID Document 解析出 service endpoint 即可发起交互，不需要预先知道 DID 背后是个人、设备、agent 还是集合。
3. **可授权**：可以作为权限主体在 RBAC / 协议层出现，既可以是 actor，也可以是 resource owner。
4. **可签名 / 签名可验证**：DID Document 中的 key、controller 或授权 agent 规则定义了“该 DID 当前由谁签名”。验证方按 DID Document 验证签名结果，不关心签名能力的内部实现。
5. **可作为 Owner / 收益主体**：可以拥有 NamedObject、内容、资产；可以被引用为版权主体、协作主体或收款主体。具体收款地址可以由 DID 绑定，也可以指向智能合约或外部分配协议。
6. **可被命名引用**：在 BNS / DID 解析层是一等公民，可以被其它实体在协议层稳定引用。

这六条是**所有实体共享**的基础。一个新引入的实体类型，必须先证明它满足这六条，才有资格成为 BuckyOS 协议中的一等主体。

关于“可签名”需要补充一点：DID 的控制权会随时间变更，例如 key 轮换、controller 迁移、host 切换。因此签名验证存在两个语义：**当前签名**按当前 DID Document 验证；**历史签名**需要按签名时的 DID Document 版本，或当时有效的凭据快照追认。这对单体和集合都成立，只是集合的 controller 规则更复杂，历史追认时需要保留更多上下文。

#### 2.1.2 集合型实体相比单体多出的能力

集合型实体在满足以上六条基础能力的前提下，额外承诺以下能力。这些能力是单体实体不具备、也不需要具备的：

1. **成员关系（双向可证）**：集合声明某个 DID 是成员，成员侧也能产生证明自己属于该集合的可验证签名。这是集合“非伪造”的根本保障。单体实体的“成员”恒为空，不存在这一语义。
2. **递归组合**：集合的成员可以是单体实体，也可以是另一个集合。BuckyOS / MessageHub 层**有且仅有这一种集合定义**；不再为团队、组织、频道、项目等语义发明独立集合实体类型，所有这类语义都通过“集合 + policy”组合表达。
3. **读取权派生于成员关系**：集合实体拥有自己的 `GROUP_INBOX`。任意一方向集合发消息时，逻辑主线只写入集合 inbox；物理投递层可以为本地 reader 创建 `INBOX` 记录、read receipt 或 tunnel delivery record，但不能把同一个 `MsgObject` 复制成多份群消息。成员通过自己作为成员的事实，获得对该集合 inbox 或其投影记录的读取权。
4. **成员变更可观察**：集合的成员是动态的，因此必须配套事件流和版本快照，使历史动作（一条消息当时投给了谁、一份签名当时由谁产生、一次收益归属当时引用了哪个集合版本）不会被后续成员变更回溯篡改。
5. **集合内部策略的部分公开**：单体实体的内部状态对外通常是黑箱；集合则必须按 policy 暴露一部分内部结构，例如成员可见性、JoinPolicy、PostPolicy、管理员可见性、nested group 策略和 attribution 策略，因为这些策略直接决定外部如何与该集合交互。

#### 2.1.3 单一集合定义的设计承诺

BuckyOS 在协议层只定义“实体”和“实体的集合”两个核心抽象。这是一个明确的、需要长期守护的设计承诺：

* 不为“组织”、“团队”、“频道”、“项目”等概念引入独立 DID 实体类型；“角色”保留为 group 内部 policy / role，不单独成为一类实体。
* 当业务上需要这些概念时，统一通过“集合 + policy + subgroup”组合表达。
* 当一个集合需要拥有独立身份、独立成员关系、独立消息历史时，它升级为一个新的集合实体（即新的 group DID），而不是发明新的实体种类。

这个承诺让 BuckyOS 的 DID 体系保持精简，同时不扩张 CYFS Protocol 的身份边界：任何一段 MessageHub 代码、任何一个授权判断、任何一次消息投递，面对的永远只有“实体”和“实体的集合”两种主体。后面 self-host group 的所有设计——DID Document 作为权限根、双向成员证明、`from=group/source=author` 的消息规范化、递归集合展开、协作作者/受益方表达、subgroup 不默认拥有独立 DID——都是在这个抽象承诺下的自然推论。

#### 2.1.4 Entity Group 是 DID Collection 的 MessageHub 实现

**DID Collection** 是协议抽象，**Entity Group** 是它在 MessageHub / GroupMgr 中的可消息化实现。Entity Group 具备三层语义：

1. **实体语义**：`group_did` 可以出现在任何接受 DID 的位置，例如 `from`、`to`、作者、权限主体、owner、controller 的被授权对象或收益归属主体。
2. **集合语义**：`group_did` 可以被 GroupMgr 解释为一组 `member_did`，成员关系由 `GroupMemberRecord` 和 `GroupMemberProof` 证明。
3. **递归语义**：`member_did` 仍然是 DID，因此它可以是 Owner/People、Device、Agent、Zone，也可以是另一个 `group_did`。

系统处理 `group_did` 时应遵循“默认不展开，按需展开”的原则：

* 作为作者、协作主体、权限主体或收益归属主体时，`group_did` 首先被当成一个完整实体。
* 只有在消息投递、成员可见性、权限继承、列表展示等需要成员级操作的场景下，才按 group policy 递归展开成员。
* 递归展开必须有最大深度、visited set 和循环检测，不能因为 A 包含 B、B 又包含 A 导致无限展开。
* 展开后的成员快照只服务于当次操作；它不能改写 group DID 作为实体的身份，也不能把集合实体永久降级成扁平成员列表。

### 2.2 Entity Group 与 Contact Collection 的区别

ContactMgr 当前有 `Contact.groups` / `tags`，这类 group 本质是本地联系人集合，用于分类、过滤或权限表达。它不拥有独立 DID，也不能作为消息的 `from/to` 主体，更不能作为内容作者、收益归属主体或被 agent 引用的协作实体。

Entity Group 是 DID Collection 的 messageable 形态，是实体而不是本地标签：

* 拥有 `group_did`
* 拥有公开可解析的 DID Document
* 可以作为 `MsgObject` 的会话对象
* 可以作为内容作者、组织、项目、家庭、团队、权限主体或收益归属主体
* 可被 ContactMgr 识别为 `is_group_did(group_did) == true`
* 可被 MessageCenter 写入 `GROUP_INBOX`
* 有成员列表、角色、可见性、邀请策略、投递策略和递归展开策略

一个业务对象如果保存了 `author_did = group_did`，它表达的是“该内容由这个 DID 集合共同创作或共同负责”。业务对象不需要先知道 group 内部有几个人、成员是否公开、收益如何分配。只有当展示作者详情、校验成员、发放收益或向成员投递通知时，才需要解析 group DID 并读取对应的集合语义。

### 2.3 Group DID 与 DID Document

Self-host group 的核心权限管理基于 DID 和 DID Document。每个 group 都可以有一个 `Group DID`，这个 DID 可以是常规二级形式，例如 `group_id.zone_id`；如果用户愿意为 group 建立一级 DID，也应允许。

这与后续 BNS 合约模型一致：BNS 合约的核心能力是把 DID 和对应 DID Document 写入智能合约，不同之处只是 DID 的控制权限和更新成本不同。无论 group DID 是二级 DID 还是一级 DID，MessageHub 侧都不应绕过 DID Document 另建一套根权限模型。

DID Document 本身视为公开信息。对 group 来说，公开内容至少包括：

1. group 的 DID。
2. group 的实体类型，例如 `entity_type=group` 或 `entity_type=did_collection`。
3. group 作者或创建者。
4. group 当前所在 Zone/OOD 信息。
5. group 后续处理入口，例如读取消息、发送消息、申请加入、提交成员签名、递归展开成员、管理员确认等 CYFS 语义路径或 KRPC/service endpoint。
6. 控制该 DID Document 的 key、controller 或授权 agent。

谁有权限控制 group DID，也由 DID Document 决定。GroupMgr 的 owner/admin 角色只能作为应用层角色；涉及 DID Document 更新、host 迁移、一级 DID 控制权变更、集合实体对外签名等根权限操作时，必须按 DID Document 的 controller 规则验证。

需要特别区分两类权限：

* **DID 控制权**：谁可以更新 group DID Document、迁移 host、签署“这个 group 加入另一个 group”的 proof。
* **集合治理权**：谁可以邀请成员、审批加入、禁言、修改 post policy、创建 subgroup。

集合治理权可以由 GroupMgr 的 Owner/Admin/Member 角色管理；DID 控制权必须回到 DID Document。二者可以由同一人持有，但协议层不能把它们混为一谈。

### 2.4 DID Document 驱动的访问流程

任意用户向 group 读取消息、发送消息、验证协作署名或展开集合成员的经典流程是：

1. 解析 `group_did` 的 DID Document。
2. 从 DID Document 获取 group 的 host Zone/OOD、service endpoint 和 CYFS 语义路径前缀。
3. 如果只是把 `group_did` 当成作者、owner、权限主体或收益归属主体，可以先把它当成普通 DID Entity 使用，不需要立即展开成员。
4. 如果操作需要成员级语义，例如读取、发送、加入群组、提交成员签名、查看公开成员、投递给成员、递归展开子 group，则调用 DID Document 指向的 GroupMgr/MessageCenter 接口；跨 Zone 写入优先表达为 `PUT cyfs://$group_ood/<group_did>/<logical_path>` 的小对象 `dispatch`。
5. host 侧再结合 GroupMgr 的成员表、成员签名、collection policy、post policy、`cyfs-original-user` / `cyfs-proofs` 等请求上下文和 MessageCenter 状态完成业务校验。

这意味着 MessageCenter/ContactMgr/GroupMgr 都是 DID Document 解析后的服务能力，不是 group 身份的来源。若 DID Document 指向的 endpoint 或语义路径前缀变更，客户端应以最新 DID Document 为准。

### 2.5 成员与管理员公开性

DID Document 是公开的，但群成员列表、管理员身份和递归成员展开结果可以选择性公开。

1. 如果选择公开成员或管理员，公开项必须能被验证，不能只是 host 单方面声明。
2. 被公开的成员关系需要双向确认：group 侧声明该 DID 是 member/admin，member 侧也能提供愿意加入该 group 的证明。
3. 实际验证方式可以是：先从 group DID Document 或公开成员索引中看到 `Alice DID`，再到 Alice 的 DID/Zone 上反向查询一条证明，证明该 DID 属于这个 group。
4. 如果公开成员本身也是另一个 `group_did`，则公开的是“子 group 作为一个实体加入父 group”的关系；除非父 group policy 明确允许递归公开，否则不应自动公开子 group 的内部成员。
5. 如果成员列表不公开，host 仍必须保存 `GroupMemberProof`，只是不把完整成员集合暴露在 DID Document 或公开索引中。

管理员身份如果具有公开治理意义，应优先公开；如果只是内部 moderation 角色，可以只在授权查询或成员可见范围内公开。

### 2.6 Hosted Group 与 Joined Group

同一个 UI 中会看到两类实体组：

* **Hosted Group**：当前 Zone 是 host。用户可以管理成员、角色、邀请、禁言、消息保留策略等。
* **Joined Group**：当前用户只是成员。它在 ContactMgr 中表现为一个外部 group 联系人，MessageCenter 可以向它发消息，但本 Zone 不拥有群成员真相源。

当前文档只描述 Hosted Group 的组件需求。Joined Group 需要保存远端 group 的展示信息和路由入口，但不维护成员真相源。

### 2.7 群消息语义

与 MessageCenter 现有设计保持一致：

* 群消息的会话主体是 `group_did`。
* 群消息作者是 `source`。
* 群消息对象本身应是 canonical JSON `NamedObject`，其 `ObjectId` 是幂等写入、目录列表和后续 Pull 的共同锚点。
* 群消息里的附件、大文件、长文本、引用内容只能放在引用字段中，例如 `ObjectId`、`FileObject`、`ChunkList` 或语义 URL，不能作为跨 Zone `dispatch` body 直接推入 host Zone。
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

跨 Zone 发送时，规范化后的消息通过 CYFS `dispatch` 投递到 group host 控制的语义路径：

```text
PUT cyfs://$group_ood/<group_did>/inbox
Content-Type: application/cyfs-named-object+json
cyfs-original-user: <author_did>
cyfs-proofs: <member_proof_or_session_proof>
<body: MsgObject canonical JSON>
```

host Zone 接受后只承诺把这个 `MsgObjectId` 纳入 group inbox 的当前目录视图。reader 后续读取 inbox 时先拿到 `ObjectId` 列表，再对缺失对象执行标准 `get_object_by_url`；看到附件引用后再按需 Pull。

### 2.8 成员资格双向确认

用户或实体成为 group member 的核心条件不是管理员单方面把 DID 写入成员表，而是同时满足：

1. group host 侧有邀请、审批或加入授权。
2. member 侧产生了有效签名，明确同意加入该 `group_did`。

签名对象应至少绑定 `group_did`、`member_did`、目标角色、邀请或申请 id、过期时间和 nonce，避免被复用到其它 group 或其它角色。

如果 member 与 group 在同一个 Zone，签名可以由本 Zone 的登录态、VerifyHub 或用户 DID 代理自动构造；但数据模型上仍应保存为可验证的 `GroupMemberProof`。这样同 Zone 自动加入和跨 Zone 手工确认使用同一套成员资格模型。

如果 `member_did` 本身也是另一个 group DID，则这条成员关系表达的是“子集合实体加入父集合实体”，而不是把子 group 的所有成员直接复制进父 group。对应的 `GroupMemberProof` 必须由子 group DID Document 中的 controller 或授权 agent 签署。是否在投递、权限判断或公开展示时递归展开子 group，由父 group 的 collection policy 和子 group 的可见性策略共同决定。

### 2.9 Group 作为语义性 DID 集合

Self-host group 不只是传统 IM 里的聊天群。更通用地说，它是一个有 DID、有公开身份、有可验证成员关系、有 MessageHub 入口的语义性 DID 集合。

因此 group 可以用于：

* 即时通讯会话。
* 通知群。
* 权限范围。
* 项目、家庭、组织、活动等实体集合。
* 内容创作、项目交付、收益归属等协作主体。
* agent 可理解和可引用的 DID 集合。

#### 2.9.1 协作与分配语义

当三个人共同创作一个内容、维护一个项目或交付一个服务时，系统可以把 `author_did`、`owner_did` 或 `beneficiary_did` 写成某个 `group_did`。这个 DID 表达“作者是这个集合实体”，而不是必须在内容对象里直接展开三个人的 DID。

这种抽象有几个好处：

1. 内容系统、交易系统、MessageHub 和 agent 都只需要处理 DID，不需要在接口层区分“个人作者”和“多人作者”。
2. group 的成员变化、公开性、分配比例和治理规则可以由 GroupMgr 或外部合约引用管理，不需要回改每个内容对象。
3. 如果收益产生后需要分配，分配系统可以通过 `group_did` 查询 group 的 attribution / revenue policy，或读取外部 `revenue_split_ref`。本文不定义具体结算协议，但 group DID 提供了稳定、可验证的协作身份锚点。
4. 如果 group 的成员不公开，外部系统仍然可以把 group DID 作为署名或收益归属主体；只有在被授权的结算、审计或成员展示场景下才展开内部成员。

#### 2.9.2 递归集合语义

BuckyOS 第一版应明确“有且仅有一种 DID 集合定义”：self-host group。一个 group 的成员是 DID，而 DID 又可以指向另一个 group。因此：

* `GroupMemberRecord.member_did` 不需要发明 `person_id`、`team_id`、`org_id` 等多套字段；它统一保存 DID。
* 当 `member_did` 是单体实体时，它表示一个 Owner/People、Device、Agent 或 Zone 成为成员。
* 当 `member_did` 是另一个 group DID 时，它表示一个集合实体成为成员。
* 父 group 是否把子 group 当成一个 opaque member，还是递归展开为子成员，应由 `GroupCollectionPolicy` 控制。
* 递归展开必须是有界操作，必须记录展开路径，必须检测循环，必须保留每一层 group 的 proof 和 policy 决策。

递归集合的关键不是“把所有成员拍平成列表”，而是让集合实体可以组合成更大的实体。例如：一个项目 group 可以包含设计 group、开发 group 和运营 group；一个家庭 group 可以包含个人 DID，也可以包含另一个小家庭 group；一个组织 group 可以把多个部门 group 作为成员。

#### 2.9.3 Group policy 决定具体使用方式

是否允许成员发消息、是否公开成员、是否暴露管理员、是否允许子 group、是否允许递归展开、是否提供 subgroup，都应作为 group policy 的一部分，而不是把 group 固定理解为“所有成员都能说话的聊天群”。

### 2.10 Subgroup

MessageHub 交互上需要支持 group 下的 subgroup。Subgroup 是父 group 内的小集合，用于更快地构建临时小组、项目小组或通知范围。

第一版 subgroup 不默认拥有独立 DID，它是 parent `group_did` 下的有名成员集合。subgroup 的成员必须已经是父 group 的 `Active` member，并且已经完成 DID 层面的双向确认。若某个 subgroup 需要脱离父 group 成为独立 messageable 实体，应升级为新的 self-host group，而不是在 subgroup 上临时发明第二套身份模型。

Subgroup 的 CYFS 语义路径建议放在父 group DID 下：

```text
PUT cyfs://$group_ood/<group_did>/sub/<subgroup_id>/inbox
GET cyfs://$group_ood/<group_did>/sub/<subgroup_id>/inbox?list=loose&after=<cursor>&limit=100
```

`subgroup_id` 只是父 group 内的路径段和状态 key，不能被当作全局 DID 使用，也不能出现在 `author_did`、`owner_did`、`beneficiary_did` 这类要求 DID 的字段里。

Subgroup 与“group 成员可以是另一个 group DID”是两种不同能力：

* subgroup 是父 group 内部的轻量视图，没有独立 DID，不能作为全局作者、owner 或收益归属主体。
* nested group 是另一个完整 DID Collection，拥有自己的 DID Document、host、policy 和成员 proof，可以作为父 group 的 member，也可以作为独立实体被其它系统引用。

---

## 3. 组件边界

### 3.1 GroupMgr

Self-host group 需要一个明确的 group 管理能力。实现上可以是 ContactMgr 内部模块，也可以独立为 `GroupMgr` 系统服务；从职责上应独立描述，避免把联系人聚合逻辑和群权威状态混在一起。

GroupMgr 负责：

1. 创建 self-host group，生成/登记 `group_did`，并把它声明为 DID Collection / Entity Group。
2. 维护 group DID Document 的应用层投影，并按 DID Document controller 规则校验根权限操作。
3. 保存 group 基本信息、成员列表、角色、设置、集合策略、版本号和可选协作归属策略。
4. 保存并验证成员加入签名，只有具备有效 `GroupMemberProof` 的成员才能进入 `Active`。
5. 支持 `member_did` 引用单体实体或另一个 group DID，并在嵌套 group 加入时校验子 group controller 的 proof。
6. 校验成员是否允许发言、邀请、管理成员、作为协作作者或参与收益分配。
7. 为 MessageCenter 提供 `is_group_did()`、`get_group_subscribers()`、`check_group_send_permission()`、`expand_group_members()`。
8. 为 ContactMgr 提供 group 联系人视图和 joined/hosted 区分。
9. 管理 subgroup，保证 subgroup 成员是父 group 的有效成员。
10. 生成成员变更、递归展开、策略变更和协作归属变更事件；对需要跨 Zone 传播或长期审计的事件，应生成 canonical JSON `NamedObject`，供 MessageHub UI、审计和后续 app 复用。
11. 为 `dispatch` ACL 提供可验证上下文，例如校验 `cyfs-original-user`、`cyfs-proofs` 中的成员证明、邀请证明、session proof 或管理员授权。

第一版建议 GroupMgr 的数据落在 MessageHub/ContactMgr 使用的 named store 中，不引入新数据库依赖。

### 3.2 ContactMgr

ContactMgr 继续负责身份和路由，但需要理解 group 是一种实体 DID。

新增或补齐能力：

1. `is_group_did(did) -> bool`：判断 DID 是否是已知 group。
2. `get_group_profile(group_did)`：返回 group 名称、头像、描述、host 信息、joined/hosted 状态、entity kind 和 collection policy 摘要。
3. `get_group_subscribers(group_did) -> Vec<DID>`：返回当前 Zone 内需要收到该群消息的 reader。对 self-host group，通常是本 Zone 内成员用户和已订阅 agent。
4. `plan_group_delivery(group_did, msg_obj, context)`：把群消息转换成面向成员或 tunnel 的小对象投递计划；如果 collection policy 允许递归展开，需要调用 GroupMgr 的展开能力并防止循环。
5. `resolve_group_member_binding(member_did)`：对远端成员，继续复用 `AccountBinding` / tunnel 选路逻辑；若成员是 group DID，应先按 policy 决定是否把它作为 opaque DID 投递，还是展开到其成员。
6. `check_group_access(group_did, actor_did, action)`：给 MessageCenter 和 UI 判断发言、邀请、踢人、修改配置等权限。
7. `resolve_did_entity_kind(did)`：在 UI、agent 和投递规划中区分未知 DID、单体实体 DID 和 group DID，但不改变 DID 本身的通用性。

ContactMgr 不应把 self-host group 简化成 `Contact.groups` 字段。`Contact.groups` 只能表示某个联系人在本地通讯录里的分类。

### 3.3 MessageCenter

MessageCenter 是群消息进入 host Zone 后的唯一消息域真相源。对 CYFS 来说，MessageCenter 挂在 group 语义路径下，决定 `dispatch` 到达的小型 `NamedObject` 是否被接受、落到哪个 inbox 目录、如何建立本地 reader 视图。

需要满足：

1. 支持 `GROUP_INBOX`，owner 为 `group_did`；其对外读取语义默认对应 `GET cyfs://$group_ood/<group_did>/inbox?list=loose`。
2. 支持 group message dispatch：校验 group 存在、作者权限、消息规范化、canonical JSON `ObjectId`、幂等写入。
3. 对每个本地 reader 创建 `INBOX` 记录或 read receipt 初始化，但不能复制 `MsgObject`；reader 视图只引用同一个 `MsgObjectId`。
4. 支持 per-reader `MsgReceiptObj`，群消息已读状态跟 reader 走，不跟 group 消息本体走。
5. 支持 host-side delivery plan：self-host group 收到群消息后，由 host 负责向远端成员投递同一个小型 `MsgObject` 或其引用；附件、Chunk、FileObject content 不随投递自动复制。
6. 当成员包含 nested group 且 policy 允许展开时，MessageCenter 应使用 GroupMgr/ContactMgr 返回的展开快照生成投递计划；不能自行递归解析成员。
7. 支持成员变更的系统消息或 event message，使群历史能解释成员加入、退出、改名、权限变化。
8. 支持把需要管理员确认的 group operation 发送到管理员个人 `INBOX`，而不只写入 group 事件流。
9. 支持把成员被移除、邀请被拒绝等面向个人的通知发送到该成员个人 `INBOX`。
10. 支持把 `group_did` 作为非聊天业务对象的通知对象，例如内容收益更新、协作任务状态变更、agent 工作流通知等；这些通知也应按 CYFS 小对象 `dispatch` + 引用 Pull 的方式设计。

关键约束：

* `MsgObject` 仍然不可变。
* 投递重试、外部 message id、tunnel 路由、失败状态仍写在 `MsgRecord.route` / `MsgRecord.delivery`。
* 群成员变更不能回改历史消息的 `to` 列表；历史消息只代表写入时的成员快照或投递计划。
* 对外 inbox 目录如果使用 `list=loose`，列表只表示 host 当前可见视图，不是可密码学证明的完整历史；需要封版审计时另行生成强一致归档对象。

### 3.4 Tunnel / Native Transport

Self-host group 需要两类投递路径：

1. **BuckyOS 原生 DID 路径**：成员也是 BuckyOS DID 时，优先通过 DID/Zone 路由投递到对方 MessageCenter。
2. **外部平台 tunnel 路径**：成员只有 Telegram/Email 等绑定时，通过 ContactMgr 选出的 tunnel 投递。

Tunnel 侧需求：

1. 能消费 `TUNNEL_OUTBOX` 中属于 group delivery 的记录。
2. 发送成功后通过 `report_delivery(record_id, result)` 回写外部消息 id 和状态。
3. 外部平台回流的群消息必须能映射到 `group_did` 和 `source`，不能只保留平台 chat id。
4. 同一外部平台群如果只是同步进来的 joined group，不应默认变成 self-host group。
5. BuckyOS 原生跨 Zone 投递优先使用 CYFS `dispatch` 语义；外部平台 tunnel 只能作为 transport adapter，不能改变 `MsgObject` 是不可变 `NamedObject`、附件按引用 Pull 的规则。

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
* `group.manage_collection_policy`
* `group.expand_members`
* `group.update_attribution_policy`
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
2. 查看 group DID、host、成员数、成员列表、角色、entity kind 和 collection policy。
3. 邀请/移除成员，并允许被邀请对象是单体 DID 或另一个 group DID。
4. 打开 group conversation。
5. 查看成员变更事件、递归成员摘要和群系统消息。
6. 对本地 reader 维护静音、置顶、归档等 per-user 设置。
7. 在个人 inbox 中处理需要管理员确认的加入申请、邀请确认和敏感变更。
8. 创建和管理 subgroup。
9. 在内容署名、项目协作、收益归属等场景中把 group DID 作为可选择实体展示。

---

## 4. 数据模型需求

### 4.1 GroupDoc

`GroupDoc` 是 self-host group 的公开或半公开描述对象，类似 `ServiceDoc` / `DeviceDoc` 这类 Doc 对象，应该可验证、可缓存。

`GroupDoc` 是 DID Document 的应用层投影或缓存，不是 DID Document 的替代品。权限判断的根仍然是 `group_did` 解析出的 DID Document；`GroupDoc` 只保存 MessageHub/GroupMgr 需要高频读取的 group profile、policy 和展示字段。

`GroupDoc` 对外发布或跨 Zone 传输时应是 canonical JSON `NamedObject`。`doc_version` 是业务版本，`ObjectId` 来自 CYFS canonical JSON Hash；同一版本内容变化后会得到新的 `ObjectId`。可选字段缺省时应省略字段，避免 `null` 与缺省产生不同 ObjectId。

建议字段：

```rust
pub struct GroupDoc {
    pub obj_type: String,          // "buckyos.group_doc"
    pub schema_version: u32,
    pub group_did: DID,
    pub entity_kind: DIDEntityKind,
    pub did_doc_id: Option<ObjId>,
    pub doc_version: u64,
    pub host_zone: DID,
    pub owner: DID,
    pub name: String,
    pub avatar: Option<String>,
    pub description: Option<String>,
    pub purpose: GroupPurpose,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub profile_version: u64,
    pub policy: GroupPolicy,
    pub collection_policy: GroupCollectionPolicy,
    pub attribution_policy: Option<GroupAttributionPolicy>,
    pub public_membership: MembershipVisibility,
    pub public_admins: MembershipVisibility,
    pub endpoints: GroupEndpoints,
    pub proof: Option<String>,
}
```

`GroupDoc` 默认不承载完整成员列表。成员列表变化频繁，应该进入 GroupMgr 的状态存储，并通过版本号、事件或摘要引用。若 group policy 选择公开成员或管理员，`GroupDoc` 可以保存公开索引或摘要，但每条公开成员关系仍需要可验证的双向证明。

`entity_kind` 对 self-host group 固定为 `DIDCollection`，用于让其它系统在解析 DID 时知道它是集合型实体。`purpose` 表达 group 的主要语义用途，便于内容系统、agent 或权限系统判断它是聊天群、通知群、协作作者组还是权限范围。`collection_policy` 描述 nested group 是否允许加入、何时递归展开以及最大展开深度。`attribution_policy` 是协作署名和收益归属的语义提示，不负责执行实际结算。

```rust
pub enum DIDEntityKind {
    SingleEntity,
    DIDCollection,
}

pub enum GroupPurpose {
    Conversation,
    Notification,
    Collaboration,
    PermissionScope,
    Organization,
    Custom(String),
}

pub enum MembershipVisibility {
    Private,
    MembersOnly,
    PublicWithProof,
}

pub struct GroupCollectionPolicy {
    pub nested_group_policy: NestedGroupPolicy,
    pub expansion_policy: GroupExpansionPolicy,
    pub max_expansion_depth: u8,
    pub reject_cycles: bool,
}

pub enum NestedGroupPolicy {
    Disallow,
    AllowAsOpaqueMember,
    AllowWithRecursiveExpansion,
}

pub enum GroupExpansionPolicy {
    NoAutoExpansion,
    ExpandForDelivery,
    ExpandForPermissionCheck,
    ExpandForPublicView,
}

pub struct GroupAttributionPolicy {
    pub attribution_mode: GroupAttributionMode,
    pub revenue_split_ref: Option<String>,
    pub split_hint: Option<Vec<GroupSplitRule>>,
    pub public_attribution: MembershipVisibility,
}

pub struct GroupSplitRule {
    pub did: DID,
    pub weight: u32,
}

pub enum GroupAttributionMode {
    OpaqueGroupDID,
    PublicMembers,
    ExternalContract,
}

pub struct GroupEndpoints {
    pub message_center: Option<String>,
    pub group_mgr: Option<String>,
    pub inbox_path: Option<String>,              // /<group_did>/inbox
    pub subgroup_inbox_prefix: Option<String>,   // /<group_did>/sub/
    pub join_path: Option<String>,
    pub submit_member_proof_path: Option<String>,
    pub expand_members_path: Option<String>,
    pub admin_operation_path: Option<String>,
}
```

`GroupEndpoints` 中的 path 是相对于 host Zone/OOD 的 CYFS 语义路径，完整访问形式由 DID Document 中的 host endpoint 决定，例如 `cyfs://$group_ood/<group_did>/inbox`。KRPC endpoint 可以作为同一能力的本地或服务间优化，但跨 Zone 互操作应优先有等价的 CYFS 语义路径。

### 4.2 GroupMemberRecord

```rust
pub struct GroupMemberRecord {
    pub group_did: DID,
    pub member_did: DID,
    pub member_kind: DIDMemberKind,
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

pub enum DIDMemberKind {
    Unknown,
    SingleEntity,
    CollectionEntity,
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

`member_kind` 是对 `member_did` 的解析缓存，不改变成员字段仍然统一为 DID。若 `member_kind=CollectionEntity`，该记录表示 nested group 作为一个成员加入父 group；是否展开取决于 `GroupCollectionPolicy`。

`Active` 状态必须满足 `member_proof_id` 指向的 `GroupMemberProof` 可验证。管理员邀请只能把成员推进到 `Invited` 或 `PendingMemberSignature`，不能直接把远端 DID 写成 `Active`。

### 4.3 GroupMemberProof

`GroupMemberProof` 是用户同意成为 group member 的签名对象。它应优先作为 canonical JSON `NamedObject` 保存和传输，也可以被 GroupMgr 投影到本地状态；关键是必须可审计、可重放验证、可绑定上下文。

```rust
pub struct GroupMemberProof {
    pub obj_type: String,          // "buckyos.group_member_proof"
    pub schema_version: u32,
    pub group_did: DID,
    pub member_did: DID,
    pub member_kind: DIDMemberKind,
    pub role: GroupRole,
    pub proof_scope: GroupMemberProofScope,
    pub invite_id: Option<String>,
    pub request_id: Option<String>,
    pub nonce: String,
    pub issued_at_ms: u64,
    pub expires_at_ms: Option<u64>,
    pub signer: DID,
    pub member_zone: Option<DID>,
    pub reverse_proof_uri: Option<String>,
    pub proof: String,
}

pub enum GroupMemberProofScope {
    JoinAsSelf,
    JoinAsCollectionEntity,
}
```

同 Zone 用户的 proof 可以自动构造，但仍应写入同样的结构。自动构造的前提是当前登录态可以代表 `member_did` 完成签名或授权，不能由管理员替成员伪造。`signer` 必须是 `member_did`，或是 `member_did` 的 DID Document 中授权过的 key/agent。

跨 Zone 提交 proof 时，调用方可以把 `GroupMemberProof` 作为 `dispatch` body 投递到 `/<group_did>/join` 或 `/<group_did>/member_proofs` 语义路径，也可以把 proof 的 `ObjectId` 放入 `cyfs-proofs`，由 host Zone 再按标准 `get_object_by_url` 拉取和校验。

当 `member_kind=CollectionEntity` 时，`proof_scope` 应为 `JoinAsCollectionEntity`，`signer` 必须是子 group DID Document 中的 controller 或授权 agent。这表示子 group 作为一个实体加入父 group，不表示子 group 内所有成员都自动对父 group 签署了成员证明。

如果 group 公开成员关系，`reverse_proof_uri` 可指向 member 自己 DID/Zone 上的一条反向证明。这样第三方可以从 group 侧和 member 侧双向验证该成员关系。

### 4.4 GroupSettings

```rust
pub struct GroupSettings {
    pub group_did: DID,
    pub join_policy: JoinPolicy,
    pub post_policy: PostPolicy,
    pub history_visibility: HistoryVisibility,
    pub retention_policy: RetentionPolicy,
    pub default_delivery: GroupDeliveryPreference,
    pub collection_policy: GroupCollectionPolicy,
    pub attribution_policy: Option<GroupAttributionPolicy>,
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
* `NestedGroupPolicy` 默认可以设为 `AllowAsOpaqueMember`：允许另一个 group DID 作为成员，但不默认递归展开。
* 需要投递或权限判断时，只有在 policy 明确允许且展开过程通过循环检测后，才递归展开 nested group。
* 新成员默认只能看到加入后的消息。
* 历史保留策略先跟 MessageCenter 默认策略一致。
* 协作署名默认使用 `GroupAttributionMode::OpaqueGroupDID`，即外部系统只看到 group DID。

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
    pub obj_type: String,          // "buckyos.group_event"
    pub schema_version: u32,
    pub event_id: String,
    pub group_did: DID,
    pub actor: DID,
    pub event_type: GroupEventType,
    pub target: Option<DID>,
    pub created_at_ms: u64,
    pub detail: Map<String, String>,
}
```

需要对外公开或长期审计的 `GroupEvent` 应保存为 NamedObject；只服务本地 UI 的临时事件索引可以是 GroupMgr 状态投影。事件流本身可以是 loose 目录，事件对象必须可独立校验。

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
* `CollectionPolicyUpdated`
* `AttributionPolicyUpdated`
* `NestedGroupAdded`
* `NestedGroupExpanded`
* `SubgroupCreated`
* `SubgroupUpdated`


### 4.7 GroupExpansionSnapshot

`GroupExpansionSnapshot` 记录一次递归集合展开的结果。它不是长期成员真相源，只用于解释某次投递、权限判断、展示或审计为什么得到某个成员集合。

```rust
pub struct GroupExpansionSnapshot {
    pub obj_type: String,          // "buckyos.group_expansion_snapshot"
    pub schema_version: u32,
    pub operation_id: String,
    pub root_group_did: DID,
    pub purpose: GroupExpansionPurpose,
    pub requested_by: Option<DID>,
    pub created_at_ms: u64,
    pub max_depth: u8,
    pub expanded_members: Vec<ExpandedDID>,
    pub opaque_members: Vec<DID>,
    pub visited_groups: Vec<DID>,
    pub truncated_groups: Vec<DID>,
    pub cycle_paths: Vec<Vec<DID>>,
    pub policy_digest: String,
    pub proof_digest: String,
}

pub enum GroupExpansionPurpose {
    Delivery,
    PermissionCheck,
    PublicView,
    Attribution,
}

pub struct ExpandedDID {
    pub did: DID,
    pub member_kind: DIDMemberKind,
    pub via_path: Vec<DID>,
    pub role: Option<GroupRole>,
}
```

同一个 group 在不同 purpose 下可以得到不同展开结果。例如投递时可能展开 nested group，但公开展示时只显示 opaque child group DID。

---

## 5. 存储与索引需求

建议使用 named store / KV 索引，保持与 MessageCenter 的 `records/{record_id}`、`box/{owner}/{box_kind}/...` 思路一致。

需要区分三类数据：

1. **可验证对象**：`GroupDoc`、`GroupMemberProof`、公开 `GroupEvent`、`GroupExpansionSnapshot`、归档消息等，应能按 CYFS canonical JSON 计算 `ObjectId`，并可通过 `get_object_by_url` 独立校验。
2. **host 当前状态**：成员表、settings、subgroup、成员索引、投递状态等，由 host Zone 的 GroupMgr/MessageCenter 维护，是权限和 UI 的当前真相源，但不是 CYFS 协议层的不可变对象。
3. **语义路径目录**：`/<group_did>/inbox`、`/<group_did>/events`、`/<group_did>/sub/<subgroup_id>/inbox` 默认是 `list=loose` 目录，返回 child `ObjectId`。需要封版时再生成强一致 container 对象。

```text
groups/{group_did}/doc -> GroupDoc
groups/{group_did}/settings -> GroupSettings
groups/{group_did}/members/{member_did} -> GroupMemberRecord
groups/{group_did}/member_proofs/{proof_id} -> GroupMemberProof
groups/{group_did}/members_by_state/{state}/{member_did} -> 1
groups/{group_did}/members_by_role/{role}/{member_did} -> 1
groups/{group_did}/nested_groups/{child_group_did} -> member_summary
groups/{group_did}/expansion_snapshots/{operation_id} -> GroupExpansionSnapshot
groups/{group_did}/attribution -> GroupAttributionPolicy
groups/{group_did}/subgroups/{subgroup_id} -> GroupSubgroup
groups/{group_did}/subgroups_by_member/{member_did}/{subgroup_id} -> 1
groups/{group_did}/events/{created_at_ms}/{event_id} -> GroupEvent
groups_by_host/{host_zone}/{group_did} -> 1
groups_by_member/{member_did}/{group_did} -> membership_summary
groups_by_child_group/{child_group_did}/{parent_group_did} -> membership_summary
objects/{object_id} -> canonical NamedObject
sem_paths/{group_did}/inbox -> loose directory backed by GROUP_INBOX
sem_paths/{group_did}/events -> loose directory backed by GroupEvent index
sem_paths/{group_did}/sub/{subgroup_id}/inbox -> loose directory backed by subgroup inbox
archives/{group_did}/{archive_id} -> strict container ObjectId
```

MessageCenter 继续维护：

```text
box/{group_did}/GROUP_INBOX/time/{sort_key}/{record_id} -> 1
box/{reader_did}/INBOX/time/{sort_key}/{record_id} -> 1
rr/{group_did}/{reader_did}/{msg_id} -> MsgReceiptObjId
```

`groups_by_member` 是 ContactMgr / UI 快速列出 joined/hosted group 的索引，不是权限真相源。权限判断必须回到 `groups/{group_did}/members/{member_did}`。

`groups_by_child_group` 用于快速查询某个 group DID 被哪些父 group 引用。它只能说明嵌套关系存在，不能代替 `GroupMemberProof`。`expansion_snapshots` 用于审计一次递归展开的输入 policy、展开路径、循环检测结果和最终成员快照，避免后续成员变化导致历史投递或权限决策不可解释。

`sem_paths/*` 是对外 CYFS 语义路径与内部索引的绑定关系，不是新的数据真相源。`GET ...?list=loose` 返回的只是当前可见 child `ObjectId` 数组；客户端必须对每个缺失对象再按 `ObjectId` 获取并校验。`archives/*` 用于需要 `cyfs-path-obj` 证明的封版历史，写入成本高，不应用在在线消息主路径。

---

## 6. 核心流程

### 6.1 创建 Self-Host Group

1. 用户在 Users & Agents 或 MessageHub 中发起创建。
2. VerifyHub/RBAC 校验当前用户是否允许创建 group。
3. GroupMgr 生成 `group_did`，可以是 `group_id.zone_id` 二级 DID，也可以是用户预先准备的一级 DID。
4. GroupMgr 创建或登记 group DID Document，写入 host Zone、controller、service endpoint、`entity_type=did_collection`、CYFS 语义路径前缀和基础公开信息。
5. GroupMgr 写入 `GroupDoc`、`GroupSettings`，默认建立 `GroupCollectionPolicy` 和 `GroupAttributionPolicy`。
6. GroupMgr 为 owner 构造或收集 `GroupMemberProof`。
7. GroupMgr 写入 owner 的 `GroupMemberRecord(role=Owner, state=Active, member_proof_id=...)`。
8. ContactMgr 建立 group entity 投影，使其出现在实体列表中。
9. MessageCenter 可选写入一条 `GroupCreated` 系统消息到 `GROUP_INBOX`。

### 6.2 邀请成员

1. actor 请求邀请 `member_did`。
2. GroupMgr 校验 actor 是否有 `group.invite_member`。
3. ContactMgr 解析 `member_did` 的实体类型，必要时创建 Shadow Contact、读取绑定或识别为另一个 group DID。
4. 如果 `member_did` 是 nested group，GroupMgr 校验父 group 的 `NestedGroupPolicy` 是否允许，并准备 `proof_scope=JoinAsCollectionEntity` 的邀请上下文。
5. GroupMgr 写入 `GroupMemberRecord(state=Invited, member_kind=...)` 和 `GroupEvent(MemberInvited)`。
6. MessageCenter 向 member 的个人 `INBOX` 或子 group 管理员的个人 `INBOX` 发送邀请消息或系统通知。
7. member 接受邀请并提交 `GroupMemberProof`。
8. 如果 group 不需要管理员复核，GroupMgr 验证 proof 后直接把成员状态置为 `Active`，写入 `MemberJoined` 事件；如果成员是 nested group，还应写入 `NestedGroupAdded` 事件。
9. 如果 group 需要管理员确认，GroupMgr 把成员状态置为 `PendingAdminApproval`，MessageCenter 向 owner/admin 的个人 `INBOX` 发送待确认消息。
10. 管理员确认后，成员状态变为 `Active`，写入 `MemberJoined` 事件。
11. 如果 member 与 group 在同一个 Zone，步骤 6-7 可以由本 Zone 在用户授权下自动完成，但仍必须生成 `GroupMemberProof`。

### 6.3 加入申请

1. member 主动请求加入 `group_did`。
2. member 提交 `GroupMemberProof`，证明自己愿意加入；如果 member 是另一个 group DID，proof 必须由该 group DID 的 controller 或授权 agent 签署。
3. GroupMgr 校验 proof、`JoinPolicy` 和 `NestedGroupPolicy`。
4. 如果 `JoinPolicy=RequestAndAdminApprove`，GroupMgr 写入 `PendingAdminApproval`，MessageCenter 向 owner/admin 的个人 `INBOX` 发送确认请求。
5. 管理员批准后写入 `Active` 成员记录；拒绝时向申请人的个人 `INBOX` 或子 group 管理员 `INBOX` 发送结果通知。

### 6.4 发送群消息

1. author 在 MessageHub/Agent 中向 `group_did` 发送消息。
2. UI/Agent 可以先提交发送意图；进入 host 写入点前，MessageCenter 规范化 `MsgObject`：`from=group_did`，`source=author_did`，`kind=group_msg`。
3. `MsgObject` 必须是 canonical JSON `NamedObject`，附件和大对象只放引用，不放跨 Zone body。
4. 如果 author 与 group host 不在同一 Zone，发送方通过 `PUT cyfs://$group_ood/<group_did>/inbox` 执行 CYFS `dispatch`，并在 `cyfs-original-user` / `cyfs-proofs` 中携带作者和成员证明上下文。
5. host MessageCenter 调用 GroupMgr 校验 `group.post_message`，校验内容包括 DID Document、成员状态、成员签名、`PostPolicy` 和 `dispatch` 请求上下文。
6. MessageCenter 按 `MsgObjectId` 幂等保存 `MsgObject`。
7. MessageCenter 写入 group 的 `GROUP_INBOX` 记录，使 `GET .../<group_did>/inbox?list=loose` 能返回该 `MsgObjectId`。
8. GroupMgr/ContactMgr 根据 `GroupCollectionPolicy` 计算本地 subscribers 和远端 delivery plan；若成员包含 nested group，必须使用有界递归展开并保存展开快照。
9. MessageCenter 根据本地 subscribers 创建 reader `INBOX` 记录或 read receipt。
10. ContactMgr 生成远端成员 delivery plan。
11. MessageCenter 写入一个或多个 `TUNNEL_OUTBOX` / native dispatch 记录，投递同一个小型 `MsgObject` 或其引用。
12. tunnel/native transport 投递后通过 `report_delivery()` 回写状态。

如果 `PostPolicy=AdminOnly`，普通 member 不能写入 group。这类 group 是通知群，管理员发送的消息仍按普通 group message 进入 `GROUP_INBOX` 和成员 inbox。

### 6.5 接收远端成员发来的群消息

1. tunnel 或 native transport 收到远端消息。
2. ContactMgr 解析外部身份到 `source_did`。
3. 如果是 BuckyOS 原生路径，native transport 应把请求视为 `dispatch` 到 `/<group_did>/inbox` 的 canonical `MsgObject`，并校验 body 可得到稳定 `MsgObjectId`。
4. GroupMgr 根据 `group_did`、`source_did`、成员 proof、`cyfs-proofs` 和 `PostPolicy` 校验成员状态。
5. MessageCenter 规范化并写入 `GROUP_INBOX`。
6. 后续流程与本地发送群消息一致。

远端消息不能绕过 host 直接写本地成员的 inbox。Self-host group 的写入点必须收敛到 host Zone 的 MessageCenter。附件不会在这一步被上传到 host 或成员 Zone；只传播引用，是否 Pull 由接收方策略决定。

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
2. reader 先读取 `GET cyfs://$group_ood/<group_did>/inbox?list=loose&after=<cursor>&limit=<n>`，得到当前可见的 `MsgObjectId` 数组。
3. reader 比对本地已持有对象，只对缺失的 `MsgObjectId` 发起标准 `get_object_by_url`，并按 CYFS canonical JSON 规则校验对象。
4. reader 展示消息时，如果发现附件、`FileObject`、`ChunkList` 或语义 URL 引用，再根据 UI、网络和权限策略按需 Pull。
5. MessageCenter 对该 reader 写入或更新独立阅读状态，可落在 reader 的 `MsgRecord.state`，也可以在 `MsgReceiptObj` 中扩展独立 `read_state` 字段。
6. read receipt 索引使用 `rr/{group}/{reader}/{msg_id}`。
7. 群消息本体和 group inbox record 不表达某个 reader 是否已读。
8. 现有 `ReceiptStatus::Accepted/Rejected/Quarantined` 表达接收结果，不应直接复用为 `READING/READED`。


### 6.10 递归集合展开

递归集合展开是一个独立能力，不应散落在 MessageCenter、ContactMgr 或 UI 中手工实现。建议由 GroupMgr 提供统一接口，并返回可审计的展开结果。

典型流程：

1. 调用方提交 `root_group_did`、`purpose`、`actor_did` 和可选 `max_depth`。
2. GroupMgr 解析 root group 的 `GroupCollectionPolicy`。
3. GroupMgr 从 root group 的 Active 成员开始遍历。
4. 如果成员是单体 DID，加入结果集合。
5. 如果成员是 nested group DID，先校验该成员关系的 `GroupMemberProof`，再读取父 group policy 和子 group 可见性策略。
6. 如果 policy 只允许 `AllowAsOpaqueMember`，则把子 group DID 作为 opaque member 加入结果，不展开其内部成员。
7. 如果 policy 允许递归展开，则解析子 group，并在 visited set 中记录路径。
8. 如果超过最大深度或发现循环，停止该分支，并在结果中记录 `truncated=true` 或 `cycle_detected=true`。
9. GroupMgr 生成 `GroupExpansionSnapshot`，包含输入、policy、路径、最终成员、被截断分支和 proof 摘要。
10. MessageCenter/ContactMgr/UI 使用这个 snapshot 做投递、权限判断或展示，不直接重新计算。

展开结果不应替代 group 的实体身份。即使某次投递把 group 展开成成员列表，内容作者、会话主体和权限根仍然是 `group_did`。

### 6.11 协作署名与收益归属

Group DID 可以作为内容或项目的协作主体。一个内容对象可以保存：

```rust
pub struct ContentAttribution {
    pub content_id: ObjId,
    pub author_did: DID,
    pub source_did: Option<DID>,
    pub attribution_context: Option<String>,
}
```

当 `author_did` 是 `group_did` 时，语义是“该内容由这个 DID 集合共同创作或共同负责”。如果某个具体成员或 agent 发起了发布，可以把它放在 `source_did` 中，类似群消息中的 `source=author_did`。

收益或责任分配不应硬编码在内容对象里。推荐方式是：

1. 内容对象只引用 `group_did`。
2. 需要展示或审计时，通过 group DID Document 找到 GroupMgr endpoint。
3. GroupMgr 返回 `GroupAttributionPolicy`、公开成员 proof 或外部 `revenue_split_ref`。
4. 结算系统或外部合约根据自己的规则执行分配，并保留当时的 group policy / member snapshot。

这样，业务系统在作者字段中看到 DID 时无需关心它是个人还是集合；只有需要分配、审计或成员展示时才展开集合。

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

    def update_collection_policy(self, actor_did, group_did, policy) -> GroupDoc: pass
    def update_attribution_policy(self, actor_did, group_did, policy) -> GroupDoc: pass
    def expand_group_members(self, group_did, purpose, actor_did=None, max_depth=None) -> GroupExpansionSnapshot: pass
    def list_parent_groups(self, child_group_did) -> list[GroupSummary]: pass

    def is_group_did(self, did) -> bool: pass
    def resolve_did_entity_kind(self, did) -> DIDEntityKind: pass
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
    def resolve_did_entity_kind(self, did) -> DIDEntityKind: pass
    def expand_group_for_delivery(self, group_did, msg_obj, context=None) -> GroupExpansionSnapshot: pass
```

### 7.3 MessageCenter API 补充

```python
class MessageCenter:
    def dispatch_group(self, group_did, msg_obj, ingress_ctx=None): pass
    def dispatch_group_named_object(self, sem_path, named_object, cyfs_headers) -> DispatchResult: pass
    def post_group_send(self, group_did, author_did, content, send_ctx=None): pass
    def post_group_admin_request(self, group_did, admin_dids, operation) -> list[MsgRecord]: pass
    def post_group_member_notice(self, group_did, member_did, notice) -> MsgRecord: pass
    def post_group_attribution_notice(self, group_did, notice, context=None) -> list[MsgRecord]: pass
    def get_group_inbox(self, group_did) -> MsgBoxHandle: pass
    def list_group_inbox_object_ids(self, group_did, cursor=None, limit=100) -> list[ObjId]: pass
    def set_group_read_state(self, group_did, msg_id, reader_did, state): pass
```

这些 API 可以复用已有 `dispatch()` / `post_send()`，但建议在接口层保留 group 语义，避免调用方手工拼错 `from/source/to/kind`。

### 7.4 CYFS 语义路径参考

后续其它 app 设计可参考同一形态：`<entity>[/<inner_logical_path>]` 表达逻辑接收点，写入用 `dispatch`，读取目录用 `GET`。

```text
PUT cyfs://$group_ood/<group_did>/inbox
GET cyfs://$group_ood/<group_did>/inbox?list=loose&after=<cursor>&limit=<n>

PUT cyfs://$group_ood/<group_did>/join
PUT cyfs://$group_ood/<group_did>/member_proofs
GET cyfs://$group_ood/<group_did>/events?list=loose&after=<cursor>&limit=<n>

PUT cyfs://$group_ood/<group_did>/sub/<subgroup_id>/inbox
GET cyfs://$group_ood/<group_did>/sub/<subgroup_id>/inbox?list=loose&after=<cursor>&limit=<n>
```

约束：

1. `PUT` body 必须是 canonical JSON `NamedObject`。
2. `PUT` 目标不能带 `/@/`。
3. `GET ...?list=loose` 返回 child `ObjectId` 数组，不内联完整对象。
4. 单次列表不超过 4096 个 child，超过时必须翻页，并通过 header 标明是否截断。
5. 需要可验证完整目录时，生成独立 archive/container `ObjectId`，不要复用在线 inbox loose 列表。

---

## 8. 安全与一致性要求

1. **DID Document 是权限根**：group 的控制权、host endpoint 和根权限必须从 `group_did` 的 DID Document 验证，GroupMgr 角色不能覆盖 DID Document controller。
2. **DID Document 公开可解析**：group 作者、当前 Zone/OOD、entity type 和 service endpoint 属于公开身份信息。
3. **DID 集合是实体，不是成员列表别名**：任何接受 DID 的业务都应先把 `group_did` 当成完整实体；只有业务需要成员级操作时才按 policy 展开。
4. **写入权限必须由 host 校验**：self-host group 的消息写入、成员变更、配置变更都必须经过 DID Document 指向的 host Zone。
5. **Active 成员必须有签名**：没有有效 `GroupMemberProof` 的 DID 不能成为 `Active` member，也不能被 subgroup 引用。
6. **nested group 必须由子 group 控制者确认**：当 `member_did` 是另一个 group DID 时，proof 必须由子 group DID Document 的 controller 或授权 agent 签署。
7. **递归展开必须有界且可审计**：展开 nested group 必须有最大深度、visited set、循环检测和 `GroupExpansionSnapshot`。
8. **公开成员关系必须双向可证**：如果公开成员或管理员列表，group 侧声明和 member 侧反向证明都必须可验证；公开 nested group 不等于公开其内部成员。
9. **同 Zone 自动签名仍需留痕**：自动构造成员签名只是交互优化，不是跳过双向确认。
10. **管理员确认进入个人 inbox**：需要管理员批准的操作必须进入 owner/admin 的个人 `INBOX`，不能只依赖 group 事件流。
11. **移除结果进入个人 inbox**：成员被移除时，必须向该成员个人 `INBOX` 发送合理通知。
12. **成员快照不可回改**：历史消息的投递范围按写入时成员状态和展开快照决定；成员退出不删除既有历史。
13. **作者与 group 分离**：`from=group_did` 表示群时间线主体，`source=author_did` 表示真实作者；在内容署名场景中，`author_did=group_did` 也应允许 `source_did` 记录具体发布者。
14. **协作署名不等于收益结算**：group DID 可以作为收益归属主体，但具体分账必须引用独立 policy、snapshot 或外部合约，不能用聊天成员列表临时推导。
15. **外部账号不等于成员身份**：Telegram/Email 账号必须先通过 ContactMgr 解析到 DID，再参与 group 权限判断。
16. **删除策略分层**：删除 group、退出 group、删除本地会话历史是不同操作；不能用一个 delete 标志表达。
17. **幂等写入**：同一个 `MsgObjectId` 重复 dispatch/post_send 不应产生重复 group inbox 或 reader inbox 记录。
18. **审计可见**：成员、权限、递归展开和 attribution policy 变化必须有事件记录，至少 host owner 可查看。
19. **遵守 CYFS No-Push**：跨 Zone `dispatch` 只能投递小型 `NamedObject` 语义事件，不能把附件、Chunk、FileObject content 或大对象作为请求体推入对方 Zone。
20. **对象可独立校验**：对外传输的 `MsgObject`、proof、event、snapshot 和归档对象必须能通过 canonical JSON 重算 `ObjectId`；host 的 loose 目录不能替代对象校验。
21. **loose 目录不承诺完整历史**：`GET ...?list=loose` 只是 host 当前可见视图；如果某个业务需要密码学可验证的完整成员集、消息归档或审计包，必须生成强一致 container / archive 对象。
22. **`dispatch` ACL 在服务层完成**：CYFS 协议不替 GroupMgr 判定成员资格。GroupMgr/MessageCenter 必须基于 DID Document、成员 proof、`cyfs-original-user`、`cyfs-proofs` 和 group policy 自行决定是否接受。
23. **语义路径与对象内路径分离**：group 写入路径只能使用 `/` 表达逻辑资源，不能对 `NamedObject` 的 `inner_path` 做写入；对象更新必须投递新对象。

---

## 9. 第一版落地范围

### 9.1 必须实现

1. 创建 self-host group，生成或登记 `group_did`，支持 `group_id.zone_id` 二级 DID 和一级 DID 两种形态。
2. 创建、登记或解析 group DID Document，并从 DID Document 获取 host endpoint 和 `entity_type=did_collection`。
3. 保存 `GroupDoc` 作为 DID Document 的应用层投影，并包含 `GroupPurpose`、`GroupCollectionPolicy` 和可选 `GroupAttributionPolicy`。
4. 保存成员表和 `GroupMemberProof`，支持 owner/admin/member/guest 等基础角色。
5. `GroupMemberRecord.member_did` 统一保存 DID，允许成员是单体实体 DID 或另一个 group DID。
6. 当成员是另一个 group DID 时，支持 `AllowAsOpaqueMember` 的 nested group 关系，并校验子 group controller proof。
7. 提供有界的 `expand_group_members()` 能力，至少支持循环检测、最大深度和展开快照；第一版可以默认不自动递归展开。
8. MessageCenter 支持 `from=group, source=author` 的群消息规范化。
9. MessageCenter 写入 `GROUP_INBOX` 和本地 reader inbox/read receipt。
10. ContactMgr 能识别 group DID，并为群成员规划投递。
11. MessageCenter 能把管理员待确认操作和成员移除通知写入个人 `INBOX`。
12. 支持 `PostPolicy::AdminOnly` 的纯通知群。
13. 支持 parent group 内的 subgroup，并明确 subgroup 不是独立 DID Collection。
14. 支持成员/管理员公开性策略，公开时保留双向 proof 或 proof URI。
15. 支持 group DID 作为内容作者、协作主体或收益归属主体的基础展示与查询语义；具体收益结算后置。
16. 支持 CYFS `dispatch` 入口：`/<group_did>/inbox`、`/<group_did>/join`、`/<group_did>/member_proofs`，并正确处理 `cyfs-original-user`、`cyfs-proofs` 和 canonical JSON `ObjectId`。
17. 支持 `GET .../<group_did>/inbox?list=loose` 返回 `MsgObjectId` 数组，reader 再按需拉取缺失 `MsgObject` 和附件引用。
18. UI 能展示 hosted/joined、member count、DID、owner、messageable、entity kind 和 collection policy 摘要。
19. tunnel 投递结果能回写到 `TUNNEL_OUTBOX` record。

### 9.2 可以后置

1. 多 host / federation 共识。
2. 多级复杂入群审批流。
3. 群公告、群文件、群内 topic/channel。
4. 全量历史同步到远端成员 Zone。
5. group 级别复杂 RBAC Schema。
6. 外部平台群到 self-host group 的自动迁移。
7. subgroup 升级为独立 Group DID 的自动迁移。
8. 复杂递归展开策略，例如跨 host 多层自动展开、继承权限合并和大规模成员缓存。
9. 内容收益结算、链上分账、版权登记和 attribution policy 的外部合约执行。
10. 一级 group DID 的 BNS 合约创建、续费和 controller 迁移 UI。
11. 群历史强一致归档、container 化封版、多源可验证归档下载。
12. `list=loose` 之外的 batch get、对象预取和离线同步优化。

---

## 10. 与现有文档的联动点

需要在实现时同步检查：

1. `Message Center.md`：群消息 `dispatch/post_send` 的伪代码应与最终 `from/source/to` 规范一致。
2. `Contact Mgr.md`：`Contact.groups` 要明确为联系人集合，不等同于 messageable group。
3. MessageHub UI model：如果当前 mock 仍使用 `from=author, to=[group]` 表达群消息，需要在协议层或 data adapter 中迁移到 `from=group, source=author`。
4. `ndn-lib::MsgObject`：确认 `source` 字段和 `kind=group_msg` 的序列化字段已经存在；如果协议对象缺字段，必须先改协议和前后端镜像类型。
5. DID/BNS 文档：确认二级 DID、一级 DID、DID Document controller、entity type、service endpoint 和 BNS 合约写入规则与本文一致。
6. DID Entity 基础协议文档：需要明确单体实体与集合型实体的共同抽象，以及 group 是第一版唯一 DID Collection 定义。
7. 内容/收益/协作对象模型：如果存在 `author_did`、`owner_did`、`beneficiary_did` 等字段，应允许其取值为 group DID，并通过 attribution policy 或外部 contract 解释分配语义。
