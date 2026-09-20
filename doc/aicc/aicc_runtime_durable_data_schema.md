# AICC Runtime Durable Data Schema

状态：Beta 2.2 当前实现契约

## 1. Overview

Service：`aicc`

本文定义 AICC 运行时中必须跨进程重启保留的 execution 幂等/恢复记录、route trace、session exact-model 历史、AICC 生成 artifact 的租户归属、Provider artifact URL 来源和 audit 事件。公共请求契约见 [aicc_api设计.md](aicc_api设计.md)，路由语义见 [aicc_router.md](aicc_router.md)。Provider inventory LKGS 由 [provider_architecture_durable_data_schema.md](provider_architecture_durable_data_schema.md) 定义，usage event 由 [aicc_usage_log_db_requirements.md](aicc_usage_log_db_requirements.md) 定义。

## 2. Data Classification

### Durable Data（持久数据）

| 数据项 | 存储 | 原因 |
|---|---|---|
| `aicc_execution_record` | AICC 平台 RDB instance | 幂等重放、内部 native task 恢复和取消必须跨重启 |
| `aicc_route_trace_event` | AICC 平台 RDB instance | 路由诊断与 task/request 关联 |
| `aicc_session_route_history` | AICC 平台 RDB instance | 保留 tenant/user/app/session 隔离的上一次 exact model 软偏好 |
| `aicc_artifact_scope` | AICC 平台 RDB instance | 记录 AICC 生成 Named Object 的租户归属，防止跨租户引用 |
| `aicc_artifact_url_source` | AICC 平台 RDB instance | 记录 Provider 返回 URL 的来源实例和租户归属，使 AICC 可鉴权后委派下载 |
| `aicc_audit_event` | AICC 平台 RDB instance | 持久安全与管理事件 |
| Provider inventory LKGS | AICC 平台 RDB instance | 见 Provider 持久 schema |
| usage event | AICC 平台 RDB instance | 见 usage log schema |
| Provider/settings | system-config | Zone 配置真相源，不复制到本 RDB |
| artifact bytes/FileObject | NDM/Named Object | RDB 只保存归属，不保存内容 bytes |

### Disposable Data（可丢弃数据）

| 数据项 | 重建方式 |
|---|---|
| RuntimeSnapshot、Adapter registry、路由索引 | 从 settings、metadata 和 inventory 重建 |
| `session_overlay` | 调用方每次请求重新传入；AICC 不持久化 |
| Provider health/quota 短期视图 | 重新 probe/query |
| resolved credential 和 Provider 短期 token | 从 locked reference 重新解析或登录 |
| task progress/delta 的运行时缓冲 | 由 task-manager event/data 重新观察，不是 AICC 最终结果真相源 |

## 3. Storage Strategy

全部结构化记录使用 `get_rdb_instance("aicc", ..., AICC_USAGE_LOG_RDB_INSTANCE_ID)` 获得的平台 RDB instance，只依赖通用 SQL/RDB 接口，不绑定 SQLite 或 PostgreSQL。AICC 生成的大结果使用 Named Object；`aicc_artifact_scope` 只是授权索引，`obj_id` 指向平台对象存储。Provider artifact URL 的内容不在登记时下载；`aicc_artifact_url_source` 只保存来源、授权范围和精确 URL，读取时交回原 ProviderInstance 的 Adapter 下载协议。

## 4. Schema Definitions

### 4.1 Table: `aicc_execution_record`

Description：按 `tenant + method + idempotency_key` 保存执行状态、pinned Provider binding 和 native task resume descriptor。

| Column | Type | Nullable | Description |
|---|---|---:|---|
| `tenant_id` | TEXT | NO | 认证 tenant |
| `method` | TEXT | NO | canonical typed/helper method |
| `idempotency_key` | TEXT | NO | 调用方幂等 key |
| `task_id` | TEXT UNIQUE | NO | AICC/task-manager task ID |
| `body_fingerprint` | TEXT | NO | 去除 idempotency key 后 canonical body 的 SHA-256 |
| `state` | TEXT | NO | 可查询的执行状态 |
| `record_json` | TEXT | NO | 完整 `ExecutionRecord`，含 pinned binding、resume descriptor、result/error |
| `created_at_ms` | BIGINT | NO | Unix ms |
| `expires_at_ms` | BIGINT | NO | 幂等窗口结束时间 |

Primary key：`(tenant_id, method, idempotency_key)`。

Indexes：

- unique `task_id`：按 task 恢复/取消。
- `idx_aicc_execution_record_state(state, created_at_ms)`：启动时查找未终结任务。
- `idx_aicc_execution_record_expiry(expires_at_ms)`：过期清理。

Constraints：同一主键不同 fingerprint 必须返回 idempotency conflict；恢复只使用已持久的 Provider/Adapter/runtime generation/credential reference，不改用当前路由。

### 4.2 Table: `aicc_route_trace_event`

Description：每次路由的脱敏结果和候选解释。

| Column | Type | Nullable | Description |
|---|---|---:|---|
| `trace_id` | TEXT PK | NO | canonical trace ID |
| `tenant_id` | TEXT | NO | tenant 隔离维度 |
| `caller_app_id` | TEXT | YES | 调用应用 |
| `task_id` | TEXT | NO | 关联 task |
| `request_id` / `route_id` / `provider_trace_id` | TEXT | YES | 跨层关联 ID |
| `request_model` | TEXT | NO | 原始逻辑或 exact model |
| `selected_exact_model` | TEXT | YES | 已选 exact model |
| `provider_instance_name` | TEXT | YES | 已选 Provider instance |
| `api_type` | TEXT | NO | canonical API type |
| `scheduler_profile` / `outcome` | TEXT | YES | 调度与结果摘要 |
| `route_trace_json` | TEXT | NO | 完整脱敏 trace |
| `created_at_ms` | BIGINT | NO | Unix ms |

Indexes：`created_at_ms`、`(tenant_id, created_at_ms)`、`(task_id, created_at_ms)`。

### 4.3 Table: `aicc_session_route_history`

Description：保存一个认证作用域内 session 上一次已选 exact model。它不是 conversation store，不保存 prompt、message、overlay、ProviderState 或 computer environment。

| Column | Type | Nullable | Description |
|---|---|---:|---|
| `tenant_id` | TEXT | NO | tenant |
| `user_id` | TEXT | NO | 认证用户 |
| `caller_app_id` | TEXT | NO | 调用 app；无 app 时使用空串 |
| `session_id` | TEXT | NO | 调用方 session key，1..512 bytes |
| `selected_exact_model` | TEXT | NO | 上一次路由已选 exact model |
| `updated_at_ms` | BIGINT | NO | 最后 upsert 时间 |

Primary key：`(tenant_id, user_id, caller_app_id, session_id)`。

Index：`idx_aicc_session_route_history_updated(updated_at_ms)`，供未来维护/清理。

Lifecycle：携带 `session_id` 的路由在所有硬约束之后把历史 exact model 作为软优先级；选路成功即 upsert 本次已选值。不携带 `session_id` 时不读写。Beta 2.2 当前没有 TTL 或自动删除。

### 4.4 Table: `aicc_artifact_scope`

Description：AICC 生成 Named Object 的授权归属索引。

| Column | Type | Nullable | Description |
|---|---|---:|---|
| `obj_id` | TEXT PK | NO | Named Object ID |
| `tenant_id` | TEXT | NO | 创建 tenant，读取授权的强制边界 |
| `user_id` | TEXT | NO | 创建用户，用于审计 |
| `caller_app_id` | TEXT | NO | 创建 app；无 app 时为空串 |
| `created_at_ms` | BIGINT | NO | Unix ms |

Index：`idx_aicc_artifact_scope_tenant(tenant_id, created_at_ms)`。

Constraints：首次创建时写入，同一 `obj_id` 不重写 owner。AICC 解析已在本表登记的对象时必须要求当前 tenant 一致。未在本表登记的外部/NDM 对象仍走 Resource 层原有授权，不默认归 AICC 租户私有。

### 4.5 Table: `aicc_artifact_url_source`

Description：AICC 对 Provider 输出中 `ResourceRef::Url` 建立的来源和授权索引。普通调用方 URL 不写入本表，AICC 不为未登记 URL 提供下载代理。

| Column | Type | Nullable | Description |
|---|---|---:|---|
| `url_hash` | TEXT PK | NO | 完整 URL 的 SHA-256，作为定长查询键 |
| `url` | TEXT | NO | Provider 返回的精确 URL；查询命中后必须再次逐字比较以防 hash 冲突 |
| `provider_instance_name` | TEXT | NO | 产生 URL 的 ProviderInstance |
| `protocol_adapter_id` | TEXT | NO | 产生 URL 时使用的 Adapter；读取时必须仍与实例匹配 |
| `artifact_id` | TEXT | YES | 可选 artifact 标识；优先取 metadata `artifact_id`，否则使用 artifact `name` |
| `tenant_id` | TEXT | NO | 创建 tenant，读取授权的强制边界 |
| `user_id` | TEXT | NO | 创建用户，用于审计 |
| `caller_app_id` | TEXT | NO | 创建 app；当前无法获得时为空串 |
| `request_id` | TEXT | NO | 产生 artifact 的请求 ID |
| `created_at_ms` | BIGINT | NO | Unix ms |

Indexes：`idx_aicc_artifact_url_source_tenant(tenant_id, created_at_ms)`、`idx_aicc_artifact_url_source_provider(provider_instance_name, created_at_ms)`。

Constraints：首次登记确定 owner 和 ProviderInstance，冲突写入不得改变现有记录。打开 reader 时必须同时满足 URL 已登记、精确 URL 相同、当前 tenant 相同、ProviderInstance 仍存在且 Adapter 未改变；调用方传入 `artifact_id` 时还必须与登记值一致。URL 可能包含 Provider 签名参数，禁止写入日志、错误消息或审计 data。

### 4.6 Table: `aicc_audit_event`

Description：脱敏审计事件。

| Column | Type | Nullable | Description |
|---|---|---:|---|
| `audit_id` | TEXT PK | NO | 稳定事件 ID |
| `tenant_id` | TEXT | NO | tenant |
| `caller_app_id` | TEXT | YES | app |
| `event_type` | TEXT | NO | 可枚举事件类型 |
| `trace_id` / `request_id` / `task_id` / `route_id` / `provider_trace_id` | TEXT | YES | 关联 ID |
| `provider_instance_name` / `exact_model` | TEXT | YES | 脱敏执行定位 |
| `data_json` | TEXT | NO | 脱敏结构化数据，禁止 credential 和请求内容 |
| `created_at_ms` | BIGINT | NO | Unix ms |

Indexes：`created_at_ms`、`(tenant_id, created_at_ms)`、`(trace_id, created_at_ms)`、`(task_id, created_at_ms)`。

## 5. Schema Version

本文表集的初始契约版本为 `1`，当前版本为 `3`。版本 2 为 usage finance 增加 `finance_amount`、`finance_currency`、`finance_valid` 查询投影；版本 3 增加 `aicc_artifact_url_source`。`finance_snapshot_json` 仍是权威数据，普通列是写入时生成且可直接信任的查询投影，读取时不反向解析 JSON，也不做双份校验。RDB 先只 bootstrap `aicc_schema_meta`，再从记录版本按 `MIGRATIONS` 的严格递增序列逐项执行事务化 migration，成功提交后才更新全局版本；高于客户端支持版本、缺少中间 migration 或执行失败均拒绝启动。业务表不再先于版本检查无条件创建。后续字段变化必须新增显式 migration，不得依赖 `CREATE TABLE IF NOT EXISTS` 猜测版本。

`usage.query` 的时间、身份、模型、Provider、method 等过滤条件、事件游标和页大小均进入 SQL；summary、group 和 time bucket 由数据库基于 typed projection 聚合，只将聚合行和请求页返回进程，不再先加载全部命中事件。

各业务表的 `*_json` 是完整权威 payload，普通 typed columns 是同一次写入生成、可直接信任的查询投影。读取列表、过滤、聚合和分页可以只读投影列，无需加载 JSON 或逐行复核两者相等；需要重建完整对象时才读取 JSON。

## 6. Upgrade Compatibility Strategy

当前为 Beta 2.2 breaking change：

| 数据项 | 策略 |
|---|---|
| `aicc_execution_record` | No-compat；进入正式发布后必须 migration，不能 rebuild 运行中任务 |
| `aicc_route_trace_event` | No-compat；将来可按 retention 清理，不可伪造重建 |
| `aicc_session_route_history` | No-compat；可丢弃时仅影响软偏好，不影响硬约束正确性 |
| `aicc_artifact_scope` | No-compat；不得从对象内容推测 tenant 重建 |
| `aicc_artifact_url_source` | No-compat；不得根据 URL host 猜测 Provider 或 owner 重建 |
| `aicc_audit_event` | No-compat；不可伪造重建 |

数据库 migration 失败时必须 error-and-stop，不得带着部分新 schema 继续接受请求。

## 7. Extensibility Rules

- Frozen：各表主键、tenant 隔离语义、idempotency fingerprint 语义、`session_id` 作用域、artifact owner 首次写入语义、URL 必须精确匹配已登记来源的语义。
- Extensible：`record_json`、`route_trace_json`、`data_json` 可增加有默认值的脱敏字段；关键查询/授权字段必须升格为 typed column，不能只藏在 JSON。
- 禁止用通用 `extra` 改变租户授权、执行恢复或 session 路由语义。
- 新列必须有明确缺省值或 migration；不允许重解释旧列。

## 8. Query Patterns

| 查询 | 索引 | 频率/边界 |
|---|---|---|
| 按 tenant/method/idempotency key 查执行 | execution PK | 每次带 key 请求，高 |
| 按 task ID 恢复/取消 | execution unique task index | 任务观察，高 |
| 启动恢复未终结任务 | execution state index | 每次启动，应并发 poll，不得被单个旧任务阻塞 |
| 按 session scope 读写上次 exact model | session PK | 每次携带 `session_id` 的路由，高 |
| 按 obj_id 校验 artifact tenant | artifact PK | 每次读 AICC 生成 Named Object，高 |
| 按 tenant/time 列出 artifact | artifact tenant index | 管理/维护，低 |
| 按 URL 打开 Provider artifact | artifact URL hash PK，再精确比较 URL | 每次读取 URL artifact，高 |
| 按 tenant/provider/time 维护 URL 来源 | artifact URL tenant/provider index | retention 与诊断，低 |
| 按 trace/task/tenant/time 查 route/audit | 对应组合索引 | 诊断，中 |
| 按 expiry 清理幂等记录 | execution expiry index | 维护，低 |

`aicc_session_route_history` 的 updated index 当前不触发自动 TTL；如果后续引入 retention，必须先在 API/路由设计中固定语义。
