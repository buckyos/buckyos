
# BuckyOS 内容发布与管理系统 (Bucky-CMS) 模块需求文档

## 1. 概述 (Overview)

### 1.1 背景

BuckyOS 系统中，用户产生的数据（Object）由 `ObjId` 唯一标识（Content-Addressable）。为了便于分享和人类记忆，需要一个映射层，将可读的“命名”映射到具体的 `ObjId`。同时，内容是动态的，需要支持对同一命名的版本迭代，并记录外部访问情况。

### 1.2 核心目标

1. **命名管理**：提供基于域名的分层命名机制，映射到 `ObjId`。
2. **版本控制 (Versioning)**：支持内容更新（Mutable Pointer），系统强制保留所有历史版本记录，不可篡改。
3. **访问审计**：聚合 `cyfs-gateway` 产生的访问数据，提供可视化的热度/流量统计。
4. **轻量级存储**：使用 SQLite 作为元数据和统计数据的存储后端。

---

## 2. 领域模型 (Domain Modeling)

### 2.1 核心实体

1. **PublishedItem (发布项)**
* **Name (Key)**: 类似于 URI 或域名（例如 `home/docs/readme.md` 或 `2025-LA-city-walk.videos`）。作为主键索引。
* **Current Pointer**: 指向当前最新版本的 `ObjId`。
* **Share Policy**: 定义内容的访问权限（如：`Public`, `TokenRequired`, `Encrypted`）。类型后期可扩展
* **Sequence**: 当前版本号（单调递增整数）。


2. **ItemRevision (版本历史)**
* 记录每一次 `PublishedItem` 的变更快照。
* 包含：`Name`, `Sequence` (Version), `ObjId`, `Timestamp`, `OpDevice` (操作设备/来源)。

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

* **创建/更新接口**：
* 输入：`Name`, `ObjId`, `SharePolicy`。
* 逻辑：
* **CAS (Compare-And-Swap) 保护**：如果是更新，建议检查前置版本号，防止并发覆盖（虽然 SQLite 是串行的，但在应用层防止逻辑冲突很重要）。
* **自动版本化**：每次更新，系统自动将`Sequence + 1`，并将旧的记录（或新记录）写入历史表。
* **不可变历史**：历史记录一旦写入，不允许修改或删除（除非执行硬性 GC 策略）。



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
    share_policy TEXT NOT NULL,         -- JSON 或 Enum: 'public', 'private', etc.
    sequence INTEGER NOT NULL DEFAULT 1,-- 当前版本号
    created_at INTEGER NOT NULL,        -- 时间戳 (ms)
    updated_at INTEGER NOT NULL         -- 最后更新时间
);

-- 2. 版本历史表 (Immutable Log)
-- 记录每一次变更，用于回滚或查看历史
CREATE TABLE item_revisions (
    name TEXT NOT NULL,
    sequence INTEGER NOT NULL,          -- 版本号
    obj_id TEXT NOT NULL,               -- 当时指向的 ObjId
    share_policy TEXT,                  -- 当时的策略
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

这里以 Rust 风格伪代码描述 Service 层的接口。

### 管理接口 (Management API)

```rust
struct PublishRequest {
    name: String,
    obj_id: String,
    policy: SharePolicy,
}

struct ItemInfo {
    name: String,
    current_obj_id: String,
    version: u64,
    history_count: usize,
    // ...
}

impl ContentService {
    /// 发布或更新内容
    /// 如果 name 不存在则创建；如果存在则创建新 Revision 并更新 Head
    fn publish(req: PublishRequest) -> Result<u64>; // 返回新的 sequence

    /// 获取当前内容指向
    fn resolve(name: &str) -> Option<String>; // 返回 ObjId

    /// 获取指定版本的历史内容
    fn resolve_version(name: &str, sequence: u64) -> Option<String>;

    /// 列出所有发布的内容（支持分页）
    fn list_items(prefix: Option<&str>, limit: usize) -> Vec<ItemInfo>;
    
    /// 获取历史记录列表
    fn list_history(name: &str) -> Vec<RevisionMetadata>;
}

// 统计服务 (供 Gateway 调用)
trait AnalyticsIngestor {
    // 写入访问日志 (通常在内存 Buffer 满时调用)
    fn record_batch(&self, logs: Vec<AccessLogEntry>) -> Result<(), CmsError>;
}

// 报表服务 (供 UI 调用)
trait AnalyticsReporter {
    // 获取聚合数据
    async fn get_stats(&self, name: &str, start_ts: u64, end_ts: u64) -> Result<Vec<TimeBucketStat>, CmsError>;
    
    // 获取原始日志 (审计用)
    async fn query_logs(&self, filter: LogFilter) -> Result<Vec<AccessLogEntry>, CmsError>;
}

```


---

## 6. 关键技术难点与解决方案

### 6.1 并发写与锁竞争 (SQLite Concurrency)

用户内容发布的处理的请求很少，主要的写压力来自access-log



---
