
# BuckyOS 内容发布与管理系统 (Bucky-CMS) 模块需求文档

## 1. 概述 (Overview)

### 1.1 背景

BuckyOS 系统中，用户产生的数据（Object）由 `ObjId` 唯一标识（Content-Addressable）。为了便于分享和人类记忆，需要一个映射层，将可读的“命名”映射到具体的 `ObjId`。同时，内容是动态的，需要支持对同一命名的版本迭代，并记录外部访问情况。

### 1.2 核心目标

1. **命名管理**：提供基于域名的分层命名机制，映射到 `ObjId`。
2. **版本控制 (Versioning)**：支持内容更新（Mutable Pointer），系统强制保留所有历史版本记录，不可篡改。
3. **上下架管理 (Enable/Disable)**：支持软上下架，下架后内容不可解析但元数据与历史保留，操作可审计。
4. **访问审计**：聚合 `cyfs-gateway` 产生的访问数据，提供可视化的热度/流量统计。
5. **轻量级存储**：使用 SQLite 作为元数据和统计数据的存储后端。

---

## 2. 领域模型 (Domain Modeling)

### 2.1 核心实体

1. **PublishedItem (发布项)** — 对应 API 结构体 `SharedItemInfo`
* **Name (Key)**: 类似于 URI 或域名（例如 `home/docs/readme.md` 或 `2025-LA-city-walk.videos`）。作为主键索引。
* **Current Pointer** (`current_obj_id`): 指向当前最新版本的 `ObjId`。
* **Share Policy**: 定义内容的访问类型。当前以字符串承载，内置常量 `public` / `token_required` / `encrypted`（见 `content_mgr_client.rs` 中的 `SHARE_POLICY_*`），后期可扩展。
* **Share Policy Config** (`share_policy_config`): 可选的 JSON 配置体，承载该策略的参数（如 token 签发方、过期时间、TTL 等），与策略类型解耦，避免每加一个参数就改类型。
* **Sequence**: 当前版本号（单调递增整数）。任何会改变 Head 状态的操作（含上下架）都会 `+1` 并落一条 Revision。
* **Enabled / 上下架状态**: `enabled` (bool) 标记该发布项当前是否对外可解析；下架时记录 `disabled_reason` 与 `disabled_at`。被禁用的项 `resolve` / `resolve_version` 一律返回 `None`，但历史与元数据仍可查。


2. **ItemRevision (版本历史)** — 对应 API 结构体 `RevisionMetadata`
* 记录每一次 `PublishedItem` 的变更快照（包括发布更新与上下架操作）。
* 包含：`Name`, `Sequence` (Version), `ObjId`, `SharePolicy`, `SharePolicyConfig`, `Enabled`(变更时的上下架状态), `CommittedAt` (Timestamp), `OpDeviceId` (操作设备/来源)。

3.**access logs**
* 保存重要的访问记录
* 会定期删除（通常保留3个月）

4. **AccessMetric (访问指标)**
* 由 `access logs` 产生的数据聚合。
* 维度：`Name`, `TimeBucket` (时间窗口)。
* 指标：`RequestCount`, `BytesSent`, `LastAccessTime`。



---

## 3. 详细功能需求 (Functional Requirements)

### 3.1 内容发布与更新 (Publishing & Mutability)

* **创建/更新接口** (`publish`)：
* 输入：`Name`, `ObjId`, `SharePolicy`, 可选 `SharePolicyConfig`, 可选 `ExpectedSequence`(CAS), 可选 `OpDeviceId`。
* 逻辑：
* **CAS (Compare-And-Swap) 保护**：更新时若传入 `expected_sequence`，系统校验其等于当前版本号（新建项要求 `expected_sequence == 0`），不匹配则拒绝，防止并发覆盖（虽然 SQLite 是串行的，但在应用层防止逻辑冲突很重要）。
* **自动版本化**：每次更新，系统自动将`Sequence + 1`，并将新记录写入历史表。
* **不可变历史**：历史记录一旦写入，不允许修改或删除（除非执行硬性 GC 策略）。

### 3.1.1 上架 / 下架 (Enable / Disable)

* **接口** (`set_item_enabled`)：输入 `Name`, `enabled` (bool), 可选 `reason`。
* 语义：软上下架，不删除任何数据。下架后 `resolve` / `resolve_version` 返回 `None`（视为不存在），但 `get_item` / `list_items` / `list_history` 仍能看到该项及其禁用原因。
* 副作用：每次上下架同样 `Sequence + 1` 并写入一条 Revision（记录当时的 `enabled` 值），保证操作可审计、可回溯。
* 配套查询：`get_item` 返回单个 `SharedItemInfo`（含 `enabled` / `disabled_reason` / `disabled_at` / `history_count`）；`is_item_enabled` 是其便捷封装。



### 3.2 命名规范 (Naming Convention)

* 支持类似文件系统的路径结构：`category/subcategory/resource_name`。
* 支持类似域名结构：`2025-LA-city-walk.videos`。
* **约束**：最大长度 256 字符，URL Safe 字符集。

### 3.3 访问统计 (Analytics Ingestion)

* **写入方**：`cyfs-gateway`。
* **写入策略**：
* 为了防止高频访问锁死 SQLite（SQLite 默认只有一把写锁），Gateway **不应**实时写入每一条请求日志。
* **Batch & Flush**：Gateway 应在内存中聚合（例如每 10 秒或每 100 次访问），批量 `UPSERT` 到 SQLite 中。



---

## 4. 数据库设计 (Schema Design - SQLite)

考虑到性能和查询便利性，建议采用以下表结构。

### 4.1 表结构 DDL

```sql
-- 1. 发布项主表 (Head State)
-- 存储当前每个名字的最新状态，用于快速解析 (Resolve)
CREATE TABLE published_items (
    name TEXT PRIMARY KEY,              -- 内容名称，如 "photos/2023/vacation"
    current_obj_id TEXT NOT NULL,       -- 当前指向的 ObjId
    share_policy TEXT NOT NULL,         -- 策略类型: 'public' / 'token_required' / 'encrypted'
    share_policy_config TEXT,           -- 可选 JSON: 策略参数 (token 签发方/过期时间/TTL 等)
    sequence INTEGER NOT NULL DEFAULT 1,-- 当前版本号
    enabled INTEGER NOT NULL DEFAULT 1, -- 上下架状态: 1=已上架可解析, 0=已下架
    disabled_reason TEXT,               -- 下架原因 (enabled=0 时有效)
    disabled_at INTEGER,                -- 下架时间 (ms)
    created_at INTEGER NOT NULL,        -- 时间戳 (ms)
    updated_at INTEGER NOT NULL         -- 最后更新时间
);

-- 2. 版本历史表 (Immutable Log)
-- 记录每一次变更 (含上下架)，用于回滚或查看历史
CREATE TABLE item_revisions (
    name TEXT NOT NULL,
    sequence INTEGER NOT NULL,          -- 版本号
    obj_id TEXT NOT NULL,               -- 当时指向的 ObjId
    share_policy TEXT,                  -- 当时的策略类型
    share_policy_config TEXT,           -- 当时的策略参数 (JSON)
    enabled INTEGER,                    -- 当时的上下架状态
    committed_at INTEGER NOT NULL,      -- 变更发生时间
    op_device_id TEXT,                  -- 操作者设备ID (审计用)
    PRIMARY KEY (name, sequence),
    FOREIGN KEY (name) REFERENCES published_items(name) ON DELETE CASCADE
);

-- 3. 原始访问日志表 (`access_logs`) - The Source of Truth
。
记录每次请求的原子事实。此表数据量大，需定期清理（TTL）。


CREATE TABLE access_logs (
    log_id INTEGER PRIMARY KEY AUTOINCREMENT, -- 自增ID，作为处理游标
    name TEXT NOT NULL,                       -- 访问的内容名
    req_ts INTEGER NOT NULL,                  -- 请求时间戳 (ms)
    source_device_id TEXT,                    -- 访问者 DeviceID (BuckyOS 身份)
    bytes_sent INTEGER DEFAULT 0,             -- 传输流量
    status_code INTEGER DEFAULT 200,          -- HTTP/RPC 状态码
    user_agent TEXT                           -- 客户端信息
);
-- 索引用于基于时间的范围查询和清理
CREATE INDEX idx_logs_ts ON access_logs(req_ts);
-- 索引用于特定内容的日志检索
CREATE INDEX idx_logs_name_ts ON access_logs(name, req_ts);


-- 4. 访问统计表 (Aggregated Metrics)
-- 按小时或天聚合，避免存储海量 Access Log
CREATE TABLE access_stats (
    name TEXT NOT NULL,
    time_bucket INTEGER NOT NULL,       -- 时间窗口，例如 unixtime / 3600 (按小时聚合)
    request_count INTEGER DEFAULT 0,    -- 访问次数
    bytes_sent INTEGER DEFAULT 0,       -- 流量消耗
    last_access_ts INTEGER,             -- 该窗口内最后访问时间
    PRIMARY KEY (name, time_bucket)
);

-- 索引优化
CREATE INDEX idx_revisions_name ON item_revisions(name);
CREATE INDEX idx_stats_time ON access_stats(time_bucket);

```

---

## 5. 接口设计 (API Specification)

服务对外以 kRPC 暴露（service id `publish-content-mgr`），客户端 `ContentMgrClient` 同时支持 `InProcess`（同进程直连 handler）与 `KRPC`（跨进程）两种接入。下方以实际结构体/方法签名描述（详见 `buckyos-api/src/content_mgr_client.rs` 与 `control_panel/src/share_content_mgr.rs`）。

### 数据结构 (Data Types)

```rust
struct PublishRequest {
    name: String,
    obj_id: ObjId,
    share_policy: String,                    // "public" / "token_required" / "encrypted"
    share_policy_config: Option<Value>,      // 策略参数 (JSON)
    expected_sequence: Option<u64>,          // CAS: 期望的当前版本号 (新建项填 0)
    op_device_id: Option<String>,            // 操作来源设备 (审计)
}

struct SharedItemInfo {
    name: String,
    current_obj_id: String,
    share_policy: String,
    share_policy_config: Option<Value>,
    enabled: bool,
    disabled_reason: Option<String>,
    disabled_at: Option<u64>,
    sequence: u64,
    history_count: u64,
    created_at: u64,
    updated_at: u64,
}

struct RevisionMetadata {
    name: String,
    sequence: u64,
    obj_id: String,
    share_policy: Option<String>,
    share_policy_config: Option<Value>,
    enabled: Option<bool>,
    committed_at: u64,
    op_device_id: Option<String>,
}
```

### 管理接口 (Management API)

```rust
impl ContentMgrClient {
    /// 发布或更新内容。name 不存在则创建；存在则创建新 Revision 并更新 Head。
    /// 返回新的 sequence。
    async fn publish(&self, request: PublishRequest) -> Result<u64>;

    /// 解析当前内容指向。若该项已下架 (enabled=false) 返回 None。
    async fn resolve(&self, name: &str) -> Result<Option<String>>; // ObjId 字符串

    /// 解析指定历史版本的内容指向。该项已下架时同样返回 None。
    async fn resolve_version(&self, name: &str, sequence: u64) -> Result<Option<String>>;

    /// 获取单个发布项的完整元数据 (含上下架状态与历史计数)。
    async fn get_item(&self, name: &str) -> Result<Option<SharedItemInfo>>;

    /// 上架 / 下架。enabled=false 时记录 reason。
    async fn set_item_enabled(&self, name: &str, enabled: bool, reason: Option<&str>) -> Result<()>;

    /// get_item 的便捷封装；项不存在时返回 Err。
    async fn is_item_enabled(&self, name: &str) -> Result<bool>;

    /// 列出发布项 (按 prefix 前缀过滤，支持 limit/offset 分页)。
    async fn list_items(&self, prefix: Option<&str>, limit: Option<usize>, offset: Option<u64>) -> Result<Vec<SharedItemInfo>>;

    /// 获取某个名字的历史记录列表 (支持分页)。
    async fn list_history(&self, name: &str, limit: Option<usize>, offset: Option<u64>) -> Result<Vec<RevisionMetadata>>;
}
```

### 统计接口 (Analytics API)

```rust
impl ContentMgrClient {
    /// 批量写入访问日志 (供 Gateway 在内存 Buffer 满时调用)。
    /// 同一事务内同时落 access_logs 与按小时聚合的 access_stats。
    async fn record_batch(&self, logs: Vec<AccessLogEntry>) -> Result<()>;

    /// 获取聚合统计。bucket_size 默认按小时；等于小时窗口时直接读 access_stats，
    /// 否则回退到对 access_logs 实时 GROUP BY 聚合。
    async fn get_stats(&self, name: &str, start_ts: u64, end_ts: u64, bucket_size: Option<u64>) -> Result<Vec<TimeBucketStat>>;

    /// 获取原始访问日志 (审计用，支持 name/device/status/时间范围/分页过滤)。
    async fn query_logs(&self, filter: LogFilter) -> Result<Vec<AccessLogEntry>>;
}
```

> 服务端实现侧对应 `ContentMgrHandler` trait（每个方法对应一个 `handle_*`），由 `ShareContentMgr` 基于 SQLite 实现；`ContentMgrServerHandler` 负责 kRPC method 名到 handler 的分发。


---

## 6. 关键技术难点与解决方案

### 6.1 并发写与锁竞争 (SQLite Concurrency)

用户内容发布的处理的请求很少，主要的写压力来自access-log



---
