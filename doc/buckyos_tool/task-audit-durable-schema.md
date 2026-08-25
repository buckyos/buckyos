# TaskManager AuditEvent 持久数据格式

## 1. Overview

Service：`task-manager`。本设计在 TaskManager 的平台 RDB instance 中增加 append-only
`audit_event`，用于 `buckyos audit query` 按 actor、action、resource 和 trace 查询系统操作事实。
协议和权限语义见 [modules/task.md](modules/task.md)。Task、TaskEvent 的既有持久格式继续由
`doc/task_mgr/task-mgr 2.0.md` 定义。

## 2. Data Classification

| Data item | Category | Reason |
| --- | --- | --- |
| `audit_event` rows | Durable | 操作事实必须跨服务重启、overlay install 和升级保留 |
| Task result 中的 diagnostic manifest | Durable | 记录采集范围、脱敏版本、hash 和 expiry |
| diagnostic/log ZIP | Disposable | 有 expiry 的派生 artifact，可以重新采集 |
| download token | Disposable | 短期 capability，进程重启后可失效 |
| KEvent notification | Disposable | 只加速读取，Task/Audit RDB 是真相源 |

## 3. Storage Strategy

`audit_event` 是结构化数据，使用 TaskManager 已有的 `task-mgr-main` 平台 RDB instance，不绑定
SQLite 或 PostgreSQL。TaskManager 的 backend-specific DDL 只表达同一逻辑 schema。

Diagnostic/log ZIP 是有界、可过期的二进制派生物，第一版放在 Control Panel cache 目录；它们
不是核心数据模型，不参与备份。Task terminal result 保存可验证 manifest。未来需要跨节点长期
保留时应迁移到 object management，不把 cache path 暴露为协议字段。

## 4. Schema Definitions

### Table: audit_event

Description：由 zone-trusted system service 追加的操作审计事实。没有 update/delete API。

| Column | Type | Nullable | Default | Description |
| --- | --- | --- | --- | --- |
| `audit_id` | TEXT PK | NO |  | `a<time>-<seq>-<random>`，opaque、近似时间有序 |
| `actor_user_id` | TEXT | NO |  | 已认证 actor user，不取自业务 payload |
| `actor_app_id` | TEXT | NO |  | 已认证 actor app/service |
| `actor_instance_id` | TEXT | YES |  | AppInstance（可得时） |
| `action` | TEXT | NO |  | 稳定动作名，如 `apps.install`、`task.cancel` |
| `resource` | TEXT | NO |  | 稳定资源引用，如 `task:<id>`；不可放 Host path |
| `trace_id` | TEXT | YES |  | 原 RPC trace id |
| `outcome` | TEXT | NO |  | `Succeeded` 或 `Failed` |
| `error_code` | TEXT | YES |  | 稳定错误码，不保存含 secret 的原始错误串 |
| `details_json` | TEXT/JSON | NO | `{}` | 已按当前 redaction version 脱敏的扩展字段 |
| `redaction_version` | INTEGER | NO | 1 | 写入 details 使用的脱敏规则版本 |
| `created_at` | INTEGER/BIGINT | NO |  | Unix timestamp milliseconds |

Indexes:

- `idx_audit_created ON audit_event(created_at, audit_id)`：默认时间分页；
- `idx_audit_actor_created ON audit_event(actor_user_id, actor_app_id, created_at, audit_id)`：
  普通用户 actor scope；
- `idx_audit_action_created ON audit_event(action, created_at, audit_id)`：action filter；
- `idx_audit_resource_created ON audit_event(resource, created_at, audit_id)`：resource filter；
- `idx_audit_trace ON audit_event(trace_id)`：按 trace 精确查询。

Constraints:

- `audit_id` 全局唯一；
- `actor_user_id`、`actor_app_id`、`action`、`resource` 非空；
- `outcome IN ('Succeeded', 'Failed')` 由协议层校验；
- `details_json` 必须解码为 JSON object；
- list cursor 是 `<created_at>:<audit_id>`，调用方视为 opaque。

### Object/Artifact: diagnostic bundle

Description：Control Panel 生成的有 expiry ZIP。  
Naming convention：`diag-<uuid>.zip`，外部只使用 `bundle_id`。  
Content format：ZIP，至少包含 UTF-8 `manifest.json` 和按 service 分隔的脱敏日志。  
Manifest fields：`schema_version`、`bundle_id`、`scope`、`redaction_version`、`sha256`、`size`、
`created_at`、`expires_at`。`sha256` 按 entry name、NUL、脱敏内容、NUL 的稳定序列计算，`size`
是脱敏内容总字节数，因此两者不依赖 ZIP 容器或 manifest 自身。Task result 保存相同内容元数据；
`diagnostic.export` 另返回短期 `artifact_sha256`，CLI 用它校验实际 ZIP bytes。

## 5. Schema Version

- `audit_event` 初始 schema version：1；
- TaskManager RDB aggregate version 由 7 提升到 8，版本保存在 service 的 RDB instance config；
- `redaction_version` 独立版本化内容脱敏规则；
- diagnostic `manifest.schema_version` 初始为 1；
- 任何列语义、cursor 排序键或 manifest frozen field 变化都提升相应版本。

## 6. Upgrade Compatibility Strategy

Beta 2.2 处于 breaking-change 开发阶段，`audit_event` 采用 **No-compat**：DV/开发数据库可由 RDB
instance 的 v8 schema 重建，不迁移不存在的旧 audit 数据。TaskMgr 2.0 的 v7 Task 表字段不在
本任务中改变。

正式发布后：

- `audit_event` 只允许 additive columns 或显式 vN→vN+1 migration；
- migration 在 TaskManager 启动、对外监听前执行；失败则服务停止，不以空 audit 表继续运行；
- diagnostic/log ZIP 是 disposable，可在升级时清理；Task result 不重写。

## 7. Extensibility Rules

- Frozen：`audit_id` identity、actor 来源、action/resource 语义、outcome 枚举、created_at、cursor
  排序规则；
- Extensible：新 nullable/defaulted columns、`details_json` 中不影响授权的字段；
- `details_json` 只能保存已经脱敏且有大小上限的 object，不得成为凭证或完整请求 payload 仓库；
- manifest 的 bundle identity、scope、hash、expiry 和 redaction version frozen；可以增加可选统计字段。

## 8. Query Patterns

| Query | Index | Cost |
| --- | --- | --- |
| 当前 actor 按时间分页 | `idx_audit_actor_created` | 高并发、index range |
| Zone 按时间分页 | `idx_audit_created` | Admin，index range |
| action + time | `idx_audit_action_created` | index range 后附加 actor scope |
| resource + time | `idx_audit_resource_created` | index range 后附加 actor scope |
| trace exact | `idx_audit_trace` | 低频、exact lookup |

第一版 actor/action/resource/trace 使用精确匹配；不提供 details 全文搜索。组合 filter 可能使用一个
索引后做有界过滤，但 `limit <= 500` 且必须带 actor scope 或 privileged Zone scope。禁止无界导出
整个 audit 表。
