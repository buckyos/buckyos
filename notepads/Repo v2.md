# RepoService 技术需求文档

> RepoService 是 BuckyOS Content Network 的核心节点组件。  
> 每个 OOD 节点运行一个 RepoService 实例。  
> 它管理两样东西：**内容本体**（节点）和**内容的传播关系**（边），构成本节点对内容网络的局部视图。

---

## 1. 定位与边界

### 1.1 RepoService 是什么

RepoService 管理的不仅是"我有哪些内容"，而是**每个内容在我这个节点视角下的网络关系图**。

- **内容本体（节点）**：每个 NamedObject 在本节点上的存在记录，包括两个层面——meta 层面（collect：我知道它存在）和实体层面（pin/store：我持有完整数据）
- **传播关系（边）**：围绕每个内容产生的可验证证明——它从哪里来、被谁收录、被谁转发、被谁下载安装。每条证明都是一条有类型、有方向、有签名的边

给定任意一个 content_id，RepoService 能回答：这个内容是谁创建的，我是从谁那里得知的，谁为它做了收录背书，谁从我这里拿走了它，谁真正安装/使用了它。

这张局部拓扑是经济模型结算（沿边分配利益）和信任模型推导（沿边传递信用）的基础数据。

### 1.2 RepoService 不做什么

| 职责 | 归属 |
|------|------|
| 内容的内部结构解析（DirObject 遍历等） | cyfs:// 协议层 |
| 内容传输、加速、分片 | cyfs:// 协议层 |
| 内容下载（断点续传、多源并行等） | Downloader 组件 |
| 内容发现 UI、评级展示、应用商店 | 上层应用 |
| BNS 名称注册与解析 | BNS 服务 |
| 购买交易、收据签发 | 链上智能合约 |
| 经济模型结算、信任评分计算 | 上层经济/信任服务（读取 proofs 数据） |

### 1.3 设计原则

- **扁平化**：objects 表是扁平的 NamedObject 记录表，不理解内容的层级结构
- **图结构**：proofs 表记录内容间的传播关系边，构成本节点的内容网络局部视图
- **原子操作**：每个写操作都是针对单个 NamedObject 或单条 proof 的原子操作
- **不碰网络传输**：所有内容实体的网络传输由协议层和 Downloader 负责，RepoService 只操作本地数据
- **meta 与实体分离**：collect（meta 入库）和 pin（实体入库）是独立的两步，中间状态由上层驱动
- **证明即边**：每次有意义的内容交互都应产生一条可验证的证明，作为关系图的边落盘

---

## 2. 核心概念

### 2.1 NamedObject 在节点上的生命周期状态

```
                        ┌─────────────────────────┐
                        │       未知 (unknown)      │
                        └────────────┬────────────┘
                                     │
                            collect(meta)
                            + 可选：referral_action
                                     │
                        ┌────────────▼────────────┐
                        │   已收录 (collected)      │
                        │   持有 meta，无实体        │
                        │   可在线消费(协议层开流)    │
                        └────────────┬────────────┘
                                     │
                          pin(content_id)
                      (前提：实体已在本地存储区)
                          + download_action
                                     │
                        ┌────────────▼────────────┐
                        │    已持有 (pinned)        │
                        │   持有 meta + 实体        │
                        │   可离线消费，可对外 serve  │
                        └─────────────────────────┘
```

对于创作者自己的内容，使用 `store` 入库，直接进入 pinned 状态并标记 origin = local。

### 2.2 Origin 标记

| origin | 含义 | 典型操作 | 删除语义 |
|--------|------|---------|---------|
| `local` | 我是这个内容的创作者/首发者 | store | 危险：可能是全网唯一副本，需警告 |
| `remote` | 我从网络上获取的别人的内容 | collect → pin | 安全：网络上还有其他持有者 |

### 2.3 访问策略

每个 NamedObject 的 meta 中声明访问策略，RepoService 执行校验：

| 策略 | 含义 | serve 时校验逻辑 |
|------|------|-----------------|
| `free` | 免费内容 | 放行，但仍产生 download action |
| `paid` | 付费买断 | 验证 receipt：签名有效 + content_name 匹配。通过后产生 download action |

收据（receipt）基于 content_name（非 content_id），因此同名内容的版本升级不需要重新购买。

### 2.4 当前 proof 表达

当前接口层已经把 proof payload 收敛为 `ndn-lib` 中的两类标准对象，而不是继续沿用自定义 proof JSON：

| 领域概念 | 当前接口类型 | 关键字段 | 说明 |
|---------|-------------|---------|------|
| **收录证明** | `InclusionProof` | `content_id` / `content_obj` / `curator` / `collection` / `rank` | 表达某个 curator 对内容的收录与背书 |
| **转发动作** | `ActionObject` | `subject` / `action=shared` / `target` / `base_on?` | 当前接口中的 `referral_action` 实际上传的是一个分享动作对象 |
| **下载动作** | `ActionObject` | `subject` / `action=download` / `target` / `base_on?` | 当前接口中的 `download_action` 实际上传的是一个下载动作对象 |
| **安装动作** | `ActionObject` | `subject` / `action=installed` / `target` / `base_on?` | 通过 `add_proof` 写入，表达真实安装/实例化 |

`ActionObject.base_on` 用于把动作串成链。例如上层可以把 `shared -> download -> installed` 组织成一条可追溯的动作链；RepoService 接口层不再把 proof 的封装格式固定死在旧的签名字段模型上。

---

### 2.5 使用标准类型

目前接口层直接使用以下标准类型：

- `ObjId`：内容与 proof 对象 ID
- `ActionObject`：转发、下载、安装等动作 proof
- `InclusionProof`：收录/背书 proof
- `PackageMeta` / `PackageId`：PKG 相关类型来自 `package-lib`

## 3. 接口设计

### 3.1 本地内容管理

#### `store(content_path) → ObjId`

创作者将自己创建的内容保存到 RepoService。

- **输入**：内容实体在本地安全存储区的路径
- **行为**：
  1. 计算内容哈希，得到 content_id（`ObjId`）
  2. 从内容中提取/构造 meta（含 owner DID、签名等）
  3. 在 repo-db 中写入 objects 记录，origin = `local`，status = `pinned`
- **输出**：`ObjId`
- **产生的证明**：无（创作者自己的内容不需要证明来源）
- **原子性**：是

#### `collect(content_meta, referral_action?) → content_id`

收录一个 NamedObject 的 metadata，不下载实体。

- **输入**：content_meta（JSON），可选的 `referral_action: ActionObject`
- **行为**：
  1. 校验 meta 中的签名有效性（owner 的 DID 公钥验签）
  2. 在 repo-db 中写入 objects 记录，origin = `remote`，status = `collected`
  3. 若提供 `referral_action`，校验后写入 proofs 表
- **输出**：content_id
- **产生的证明**：落盘传入的分享动作（若有）
- **原子性**：是
- **幂等性**：重复 collect 同一 content_id 不报错，更新 meta（如果版本更新）

#### `pin(content_id, download_action) → bool`

将已 collect 的内容标记为本地持有（前提：实体已由 Downloader 下载到本地存储区）。

- **输入**：content_id，`download_action: ActionObject`
- **前置条件**：
  1. repo-db 中已有该 content_id 的 collected 记录
  2. 本地安全存储区中已存在该 content_id 对应的实体文件
- **行为**：
  1. 校验本地实体的哈希与 content_id 一致
  2. 校验 `download_action` 后写入 proofs 表
  3. 更新 repo-db objects 记录 status = `pinned`
- **输出**：成功/失败
- **产生的证明**：落盘 download action
- **原子性**：是

#### `unpin(content_id, force=false) → bool`

释放本地实体，回到仅持有 meta 的状态。

- **输入**：content_id，`force`
- **行为**：
  1. 若 origin = `local`，返回警告（可能是全网唯一副本），需 force 参数确认
  2. 删除本地存储区中的实体文件
  3. 更新 repo-db objects 记录 status = `collected`
- **输出**：成功/失败
- **产生的证明**：无（proofs 记录保留，不随 unpin 删除）
- **原子性**：是

#### `uncollect(content_id, force=false) → bool`

彻底移除一个 NamedObject，包括 meta 和实体（如有）。

- **输入**：content_id，`force`
- **行为**：
  1. 若 status = `pinned`，先执行 unpin 逻辑
  2. 从 repo-db objects 表中删除记录
  3. **proofs 表中的相关记录保留**（历史传播关系不应因 uncollect 而丢失，经济结算可能仍需要）
- **输出**：成功/失败
- **原子性**：是

### 3.2 证明管理

#### `add_proof(proof) → proof_id`

直接写入一条证明。用于非 collect/pin 流程中产生的动作或收录背书（如安装动作、外部生成的 `InclusionProof`）。

- **输入**：`proof: RepoProof`
- **行为**：
  1. 根据 proof 的具体类型做对象级校验
  2. 写入 proofs 表
- **输出**：proof_id
- **原子性**：是

#### `get_proofs(content_id, filter?) → [RepoProof]`

查询某个内容相关的所有证明（边）。

- **过滤条件**：当前接口保留 `proof_type / from_did / to_did / start_ts / end_ts` 这组兼容字段；具体实现会把它们映射到 `ActionObject.action / subject / target` 或 `InclusionProof.curator`
- **输出**：该内容的所有相关 proof 列表


### 3.3 查询接口



#### `resolve(content_name) → [ObjId]`

根据 `content_name` 查询**本地 Repo 视角下已 pinned 的对象 ID 列表**。它不负责从 BNS 或远端拉取 meta。

#### `list(filter?) → [repo_record]`

列出本地所有记录。支持过滤条件：

| 过滤字段 | 说明 |
|---------|------|
| `status` | `collected` / `pinned` |
| `origin` | `local` / `remote` |
| `content_name` | 按名称前缀匹配 |
| `owner_did` | 按 owner 过滤 |

#### `stat() → repo_stat`

返回仓库统计信息：总记录数、collected 数量、pinned 数量、本地实体总大小、证明总数等。

### 3.4 对外服务

#### `serve(content_id, request_context) → RepoServeResult`

响应来自 Zone 内或 Zone 外的内容请求。

- **输入**：
  - content_id：请求的内容
  - request_context：包含请求者 DID、携带的 receipt（可选）
- **行为**：
  1. 查找 repo-db，若不存在或 status ≠ `pinned`，返回 reject（NOT_FOUND）
  2. 读取 meta 中的访问策略
  3. 若 `free`：放行
  4. 若 `paid`：验证 receipt —— 签名有效 + receipt.content_name 与 meta.content_name 匹配
  5. 校验通过后，生成 `download_action: ActionObject`，返回结构化的 `RepoServeResult`
  6. 将 `download_action` 写入本节点 proofs 表
- **输出**：`RepoServeResult { status, content_ref?, download_action?, reject_code?, reject_reason? }`
- **产生的证明**：download action

### 3.5 发布辅助

#### `announce(content_id) → bool`

将本地 store 的内容的 meta 发布到 BNS 智能合约。

- **前置条件**：origin = `local`，content_name 已绑定
- **行为**：调用 BNS 服务，将 content_meta 写入链上
- **说明**：这是对 BNS 服务的封装调用，RepoService 自身不实现链上逻辑

---

## 4. repo-db Schema 设计

### 4.1 内容表：`objects`

每行代表一个 NamedObject 在本节点上的存在记录（图的节点）。

```sql
CREATE TABLE objects (
    -- 主键：内容寻址 ID
    content_id      TEXT PRIMARY KEY,

    -- 名称标识（可选，格式如 did:bns:app1.alice）
    content_name    TEXT,

    -- 状态：collected / pinned
    status          TEXT NOT NULL DEFAULT 'collected',

    -- 来源：local（自己创建）/ remote（从网络获取）
    origin          TEXT NOT NULL,

    -- 内容 metadata（完整 JSON，RepoService 不解析 payload 部分）
    -- 通用信封字段：owner_did, author, signature, content_name, price
    -- payload 部分：由上层应用自由定义
    meta            TEXT NOT NULL,

    -- === 以下为从 meta 中提取的索引字段 ===

    -- 版权所有者 DID
    owner_did       TEXT,

    -- 作者（可为空，为空时等于 owner）
    author          TEXT,

    -- 访问策略：free / paid
    access_policy   TEXT NOT NULL DEFAULT 'free',

    -- 价格（access_policy = paid 时有意义）
    price           TEXT,

    -- === 本地存储信息 ===

    -- 本地实体文件路径（status = pinned 时非空）
    local_path      TEXT,

    -- 实体大小（字节）
    content_size    INTEGER,

    -- === 时间戳 ===

    collected_at    DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    pinned_at       DATETIME,
    updated_at      DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
```

### 4.2 证明表：`proofs`

每行代表一条传播关系（图的边）。这是经济模型和信任模型的基础数据。

```sql
CREATE TABLE proofs (
    -- 证明 ID（哈希或 UUID）
    proof_id        TEXT PRIMARY KEY,

    -- 关联的内容
    content_id      TEXT NOT NULL,

    -- proof 类型
    -- action:      ActionObject(shared/download/installed/...)
    -- collection:  InclusionProof
    proof_kind      TEXT NOT NULL,

    -- ActionObject 的常见索引字段
    action_type     TEXT,
    subject_id      TEXT,
    target_id       TEXT,
    base_on         TEXT,

    -- InclusionProof 的常见索引字段
    curator_did     TEXT,

    -- proof 原始对象 JSON
    proof_data      TEXT NOT NULL,

    -- 创建时间
    created_at      DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
```

### 4.3 收据表：`receipts`

记录本节点持有的购买收据，用于 serve 时校验。

```sql
CREATE TABLE receipts (
    -- 收据 ID（链上交易 ID 或本地签发 ID）
    receipt_id      TEXT PRIMARY KEY,

    -- 购买的内容名称（基于 name 而非 id，升级不需重新购买）
    content_name    TEXT NOT NULL,

    -- 购买者 DID
    buyer_did       TEXT NOT NULL,

    -- 卖方 DID（即内容 owner）
    seller_did      TEXT NOT NULL,

    -- 收据签名
    signature       TEXT NOT NULL,

    -- 收据原始数据（完整 JSON）
    receipt_data    TEXT NOT NULL,

    created_at      DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
```

### 4.4 索引

```sql
-- objects 索引
CREATE INDEX idx_content_name ON objects(content_name);
CREATE INDEX idx_status ON objects(status);
CREATE INDEX idx_origin ON objects(origin);
CREATE INDEX idx_owner_did ON objects(owner_did);
CREATE INDEX idx_collected_at ON objects(collected_at);

-- proofs 索引
CREATE INDEX idx_proof_content_id ON proofs(content_id);
CREATE INDEX idx_proof_kind ON proofs(proof_kind);
CREATE INDEX idx_proof_action_type ON proofs(action_type);
CREATE INDEX idx_proof_subject ON proofs(subject_id);
CREATE INDEX idx_proof_target ON proofs(target_id);
CREATE INDEX idx_proof_curator ON proofs(curator_did);
CREATE INDEX idx_proof_created ON proofs(created_at);
CREATE INDEX idx_proof_content_kind ON proofs(content_id, proof_kind);

-- receipts 索引
CREATE INDEX idx_receipt_content_name ON receipts(content_name);
CREATE INDEX idx_receipt_buyer ON receipts(buyer_did);
```

---

## 5. 接口与 DB 操作映射

| 接口 | objects 表 | proofs 表 | receipts 表 | 产生的证明 |
|------|-----------|-----------|-------------|-----------|
| `store(path)` | INSERT (origin=local, status=pinned) | — | — | 无 |
| `collect(meta, referral_action?)` | INSERT (origin=remote, status=collected) | INSERT shared action（若有） | — | referral_action |
| `pin(id, download_action)` | UPDATE status=pinned | INSERT download action | — | download_action |
| `unpin(id)` | UPDATE status=collected + 删除文件 | 不删除 | — | 无 |
| `uncollect(id)` | DELETE | 不删除（历史保留） | — | 无 |
| `add_proof(proof)` | — | INSERT | — | 传入的 proof |
| `serve(id, ctx)` | SELECT 校验 pinned | INSERT download action | SELECT 校验 receipt | download_action |
| `get_proofs(id)` | — | SELECT WHERE content_id=? | — | — |

---

## 6. 证明的生命周期

### 6.1 证明的持久性

证明一旦写入 proofs 表，**不随内容的 unpin 或 uncollect 而删除**。理由：

- 经济结算可能在内容被移除后仍需回溯传播链
- 信任模型需要历史行为数据来建立信用
- 证明是独立于内容存在的事实记录

### 6.2 证明的可导出性

proofs 表的数据可被上层经济服务和信任服务读取，用于：

- 按传播链分配内容收益（owner → 收录者 → 传播者）
- 计算节点的内容分发贡献（download action 的数量与质量）
- 推导节点信用评分（真实安装 vs 刷量识别，基于证明的交叉验证）

### 6.3 防刷设计基础

当前 proof 对象的可验证性为防刷提供了基础：

- **收录证明**由 `InclusionProof` 表达 curator 背书，刷收录需要伪造或滥用 curator 身份
- **转发动作**由分享者创建，刷转发需要控制大量真实传播节点
- **下载动作**可通过 `base_on` 与传播/安装动作交叉验证，降低孤立刷量的价值
- **安装动作**可结合真实实例化与后续使用行为做交叉验证

具体的反刷策略由上层信任服务实现，RepoService 只负责如实记录。

---

## 7. 与系统其他组件的交互

```
┌─────────────────┐  collect/pin/store    ┌──────────────────┐
│    上层应用       │ ◄──────────────────► │   RepoService    │
│  (AppStore,      │                      │                  │
│   VideoApp,      │  get_proofs / list   │  ┌────────────┐  │
│   UI ...)        │ ◄──────────────────► │  │  objects    │  │
└─────────────────┘                      │  │  proofs     │  │
                                         │  │  receipts   │  │
┌─────────────────┐  get_proofs          │  └────────────┘  │
│  经济模型服务     │ ◄──────────────────► │                  │
│  信任模型服务     │  读取证明数据         └────────┬─────────┘
└─────────────────┘                               │
                                    content_ref   │ 本地文件读取
                                    + proofs      │
                                                  ▼
┌─────────────────┐  下载完成          ┌───────────────────────┐
│   Downloader    │ + download_action │   本地安全存储区         │
│                 │ ──────────────►   │                       │
└────────┬────────┘                  └───────────────────────┘
         │                                      ▲
         │  基于 content_id                      │ open_stream
         │  从网络拉取实体                        │
         ▼                                      │
┌─────────────────┐                   ┌─────────┴────────┐
│  cyfs:// 协议层   │ ◄───────────────► │   内容消费者       │
│  (传输/加速)      │   流式传输         │  (播放器等)       │
└─────────────────┘                   └──────────────────┘
```

**典型流程与证明产生：**

**创作者发布内容：**
store(local_path) → announce(content_id) → 无证明（自己的内容）

**消费者发现内容（通过社交分享）：**
A 分享给 B → A 创建 `ActionObject(action=shared)` → B 调用 `collect(meta, referral_action)` → shared action 落盘

**消费者下载并安装应用：**
downloader.download(content_id) → 传输完成生成 `ActionObject(action=download)` → `pin(content_id, download_action)` → download action 落盘
→ app.create_instance(spec) → 生成 `ActionObject(action=installed, base_on=...)` → `add_proof(install_action)` → install action 落盘

**收录者背书内容：**
外部生成 `InclusionProof` → `add_proof(RepoProof::Collection(...))` → collection proof 落盘
