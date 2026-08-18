# AICC 视频续接来源索引持久化格式

## 1. Overview

服务：AICC

AICC 的 Task Manager 任务结果已经持久保存生成结果、最终模型及 provider 返回的续接信息。本设计只增加从视频内容标识到原 Task Manager 任务的反向索引，使同一视频在重新上传、对象 ID 改变或会话清理后，仍能读取原任务结果并恢复原生续接。索引不重复保存 provider handle、模型或任务结果。

相关接口与实现见 `doc/aicc/aicc_api设计.md`、`src/frame/aicc/src/aicc.rs` 和 `src/kernel/buckyos-api/src/taskdata.rs`。

## 2. Data Classification

### Durable Data（持久数据）

| 数据项 | 分类原因 |
|---|---|
| 视频内容标识到原 Task Manager task ID 的索引 | 需要跨进程重启、会话清理和附件重新上传继续使用 |
| AICC 任务结果 | 已由 Task Manager 持久保存，是续接信息和模型信息的唯一真相源 |
| 视频和 FileObject | 已由 Named Store 持久保存，不在本索引中重复存储 |

### Disposable Data（可丢弃数据）

| 数据项 | 分类原因 |
|---|---|
| 单次请求解析得到的视频内容标识 | 可从视频字节重新计算 |
| 当前路由候选和执行进度 | 由现有任务与路由机制管理 |
| CLI 下载到本地的媒体副本 | 仅用于当前工具调用，可从 Named Store 或附件重新读取 |

## 3. Storage Strategy

反向索引属于结构化数据，存入 AICC 已有的平台 RDB 实例 `aicc-usage-log`，不依赖具体 SQLite 或 PostgreSQL 行为。任务结果继续由 Task Manager 管理，媒体继续由 Named Store 管理；索引不保存文件路径、媒体字节、provider handle 或模型副本。

## 4. Schema Definitions

### Table: aicc_video_continuation_source

Description：将可原生续接的视频内容关联到保存完整生成结果的 Task Manager 任务。

| Column | Type | Nullable | Default | Description |
|---|---|---|---|---|
| tenant_id | TEXT | NO | | 租户隔离键 |
| content_id | TEXT | NO | | 视频字节计算得到的稳定 chunk/content ID；对 AICC 生成的单内容 FileObject 等于其 `content` 字段，不是包含文件名和元数据的 FileObject `obj_id` |
| source_task_id | TEXT | NO | | 可由 Task Manager `get_task` 直接读取的内部 task ID |
| created_at_ms | INTEGER/BIGINT | NO | | 索引创建或替换时间，Unix 毫秒 |

Indexes:

- Primary key `(tenant_id, content_id)`：同一租户和视频内容只保留最近一次可用来源任务。
- `idx_aicc_video_continuation_source_task(source_task_id)`：按来源任务审计或清理索引。

Constraints:

- 不允许跨 tenant 查询来源任务。
- `content_id` 必须来自实际视频字节，不使用文件名、FileObject ID 或本地路径。
- `source_task_id` 必须是 Task Manager 内部 task ID，不是 AICC 外部任务号。
- handle、模型、provider 信息只从来源任务结果读取，不写入本表。

## 5. Schema Version

- 初始版本：4，与 `AICC_USAGE_LOG_RDB_SCHEMA_VERSION` 一致。
- 版本保存在系统服务 spec 的 RDB instance 配置中。
- 后续表结构变化递增该版本。

## 6. Upgrade Compatibility Strategy

当前 beta 2.2 为 breaking change，采用 No-compat 策略。DDL 使用 `CREATE TABLE IF NOT EXISTS` 创建索引表，不迁移旧任务；旧视频没有索引、来源任务已被清理或任务结果不含续接信息时，原生续接不可用，可以向用户说明后采用替代方案。

## 7. Extensibility Rules

- Frozen：`tenant_id`、`content_id` 和 `source_task_id` 的语义。
- Extensible：未来可以增加来源失效时间、最近访问时间或清理状态。
- 不增加通用 JSON `extra` 列；续接状态仍属于 Task Manager 任务结果。

## 8. Query Patterns

| 查询 | 索引 | 频率 |
|---|---|---|
| 根据 tenant 和视频 content ID 获取来源任务 | Primary key | 高 |
| 生成任务完成时写入或替换来源任务 | Primary key | 每个可续接视频一次 |
| 按来源任务审计或删除索引 | `idx_aicc_video_continuation_source_task` | 低 |

不存在运行时全表扫描。查询到来源任务后，通过 Task Manager `get_task(source_task_id)` 读取既有任务结果；来源任务不可用时不得从索引推测或重建 provider 私有状态。
