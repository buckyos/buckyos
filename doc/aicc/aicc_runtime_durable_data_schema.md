# AICC Runtime Durable Data Schema

状态：Beta 2.2 当前实现契约

## 1. Overview

Service：`aicc`

本文定义 AICC 运行时中必须跨进程重启保留的 execution 幂等/恢复记录、route trace、用途表的用户层 / 路由策略组 / App 绑定、AICC 生成 artifact 的租户归属、Provider artifact URL/原生 ID 来源。公共请求契约见 [aicc_api设计.md](aicc_api设计.md)，路由语义见 [aicc_router.md](aicc_router.md)。Provider inventory LKGS 由 [provider_architecture_durable_data_schema.md](provider_architecture_durable_data_schema.md) 定义，usage event 由 [aicc_usage_log_db_requirements.md](aicc_usage_log_db_requirements.md) 定义。

## 2. Data Classification

### Durable Data（持久数据）

| 数据项 | 存储 | 原因 |
|---|---|---|
| `aicc_execution_record` | AICC 平台 RDB instance | 幂等重放、内部 native task 恢复和取消必须跨重启 |
| `aicc_route_trace_event` | AICC 平台 RDB instance | 路由诊断与 task/request 关联 |
| `aicc_routing_layer` | AICC 平台 RDB instance | 用户层：每个用户自己的用途表配置（[aicc_router.md §11.2](aicc_router.md#112-层次与存放)） |
| `aicc_routing_group` | AICC 平台 RDB instance | 路由策略组：属于建它的用户 |
| `aicc_routing_binding` | AICC 平台 RDB instance | 用户把自己的策略组绑定到 App 的记录 |
| `aicc_artifact_scope` | AICC 平台 RDB instance | 记录 AICC 生成 Named Object 的租户归属，防止跨租户引用 |
| `aicc_artifact_url_source` | AICC 平台 RDB instance | 记录 Provider 返回 URL 的来源实例和租户/用户归属，供直传判定与鉴权下载；配置通过来源实例读取，不另存 endpoint 快照 |
| `aicc_provider_artifact` | AICC 平台 RDB instance | 按内容摘要**或**对象 ID、ProviderInstance、模型原厂和 tenant 保存图片、视频、音频等 artifact 的原生 ID；纯缓存 |
| Provider inventory LKGS | AICC 平台 RDB instance | 见 Provider 持久 schema |
| usage event | AICC 平台 RDB instance | 见 usage log schema |
| Provider/settings | system-config | Zone 配置真相源，不复制到本 RDB；含用途表的管理员层 `services/aicc/settings.routing.usecases` 和 Provider 的 `marked_faulty` |
| artifact bytes/FileObject | named store + AICC 桶 `user/aicc/<user>/outputs`、`inputs` | RDB 只保存归属，不保存内容 bytes；桶绑定是唯一的保留（`temp`，7 天 / 1 天） |

### Disposable Data（可丢弃数据）

| 数据项 | 重建方式 |
|---|---|
| RuntimeSnapshot、Adapter registry、路由索引 | 从 settings、metadata 和 inventory 重建 |
| 会话绑定的策略组 | 由 lib_opendan 保存在会话配置 `route_group` 中，调用时随请求传入；AICC 不持久化 |
| Provider health/quota 短期视图 | 重新 probe/query |
| resolved credential 和 Provider 短期 token | 从 typed credential source 重新解析或登录 |
| task progress/delta 的运行时缓冲 | 由 task-manager event/data 重新观察，不是 AICC 最终结果真相源 |

## 3. Storage Strategy

全部结构化记录使用 `get_rdb_instance("aicc", ..., AICC_USAGE_LOG_RDB_INSTANCE_ID)` 获得的平台 RDB instance，只依赖通用 SQL/RDB 接口，不绑定 SQLite 或 PostgreSQL。AICC 生成的大结果使用 Named Object，写入即绑定（见下表，契约见 [aicc_api设计.md](aicc_api设计.md) §3.1 物化即保留）；`aicc_artifact_scope` 只是授权索引，`obj_id` 指向平台对象存储。Provider artifact URL 的内容不在登记时下载；`aicc_artifact_url_source` 保存来源、授权范围、精确 URL、模型原厂和 nullable 内容摘要，读取时交回原 ProviderInstance 的 Adapter 下载协议，并在完整读到 EOF 后补全摘要。`aicc_provider_artifact` 不保存内容，只把 Provider 原生 ID 关联到内容摘要**或**对象 ID，作用域为 ProviderInstance + 模型原厂 + tenant。

### Unstructured：应用对象桶

`services/aicc/spec` 声明两个 user scope 桶（`buckyos_api::aicc_bucket_decls`，app 池 `storage_quota_mib = 8192`，按用户计）：

| 桶 | kind / data_policy | key | 谁写、谁 pin | 谁读 |
|---|---|---|---|---|
| `user/aicc/<user>/outputs` | `data` / `temp`，`default_ttl_days = 7` | `<app>/<hash16>/<name>` | `ResourceManager::write_artifact` 写 chunk + FileObject 后立即 `bucket.put`（无条件写，同 key 同对象即刷新 TTL）；pin owner `bucket:<bucket_id>` | 该用户；业务 App 经 AICC 授的 `<app>/` 前缀只读 grant；frame 服务 |
| `user/aicc/<user>/inputs` | `data` / `temp`，`default_ttl_days = 1` | `<app>/<hash16>/<name>`（Desktop 上传：`<hash16>/<文件名>`） | `ResourceManager::write_input`、只收 URL 的 Provider 的 inline 输入发布、浏览器上传 finalize 的 `auto_bind` | 同上 |

调用方要长期保留结果就 `bucket.copy` 到自己的桶（OpenDAN/Agent：`user/<agent>/<owner>/artifacts`）。`aicc:input:*` Lease 与 `data/aicc/driver_metadata/cloud`（cache 分区，可重建）不变。

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
| `record_json` | TEXT | NO | 完整 `ExecutionRecord`，含 pinned binding、resume descriptor、result/error；native URL 输入可在 resume descriptor 中保存 canonical request 与 fallback target（bytes 或提交时配置的公网 Named Object base URL），重提后只替换远端 task id，不改变本地 task id |
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
| `route_trace_json` | TEXT | NO | 完整脱敏 trace，含 `usecase`（来源层、命中的策略组）与 `fallback`（[aicc_router.md §13.2](aicc_router.md#132-route-trace-schema)） |
| `created_at_ms` | BIGINT | NO | Unix ms |
| `fallback_applied` | INTEGER | NO | 过滤投影，默认 0。应在 trace 带 `fallback`（由兜底 LLM 提供或按“不可用时”策略切换）时为 1；当前写入代码仍读取 trace 里已删除的 `fallback_applied` 字段，因此总是 0 |
| `warning_count` | BIGINT | NO | trace `warnings` 的条数 |

Indexes：`created_at_ms`、`(tenant_id, created_at_ms)`、`(task_id, created_at_ms)`、`(tenant_id, fallback_applied, created_at_ms)`、`(tenant_id, warning_count, created_at_ms)`。

### 4.3 已删除：`aicc_session_route_history`

2026-11-28 删除（router N2）：路由不再按会话历史软优先，请求也没有 `session_id`。schema 4 的新库不建这张表；从 schema 3 升级时把它删掉，不迁移。

### 4.4 Table: `aicc_routing_layer`

Description：用户层（[aicc_router.md §11.2](aicc_router.md#112-层次与存放)）。每个用户一行，内容是“用途 → 模型 + 不可用时策略”的完整配置。

| Column | Type | Nullable | Description |
|---|---|---:|---|
| `subject` | TEXT PK | NO | `user:<user_id>`，与 policy 表的主体写法一致 |
| `content_json` | TEXT | NO | `AiccUsecaseRoutes`（`buckyos-api/src/aicc_routing.rs`）：`{"<usecase>": {"model"?, "on_unavailable"?}}`；用户层不含 `locked`、`enabled` |
| `version` | BIGINT | NO | 每次写入加 1；`routing.usecase.set {target: "user"}` 的 `revision` 必须等于它，没有这一行时为 0 |
| `updated_at_ms` | BIGINT | NO | Unix ms |

Constraints：只经 `routing.usecase.set` 写入，写入前按 [aicc_router.md §11.1](aicc_router.md#111-用途与配置格) 校验；用户只能读写自己的一行。所选模型后来消失时内容保留，求值时这一格跟随下一层。

### 4.5 Table: `aicc_routing_group`

Description：路由策略组（[aicc_router.md §12.2](aicc_router.md#122-路由策略组与-scope)），内容与用户层同形状。

| Column | Type | Nullable | Description |
|---|---|---:|---|
| `group_id` | TEXT PK | NO | 服务生成，前缀 `rg_` |
| `owner` | TEXT | NO | `user:<user_id>`，建组的用户；组只对它生效 |
| `name` | TEXT | NO | 1–64 个字符 |
| `content_json` | TEXT | NO | `AiccUsecaseRoutes`，同 `aicc_routing_layer` |
| `version` | BIGINT | NO | 每次写入加 1；`routing.groups.update` / `.delete` 与 `routing.usecase.set {target: "group:<id>"}` 必须带当前值 |
| `updated_at_ms` | BIGINT | NO | Unix ms |

Index：`idx_aicc_routing_group_owner(owner)`。

Constraints：读写都按 `owner` 限定；请求里的 `route_group` 不属于调用者的额度用户时当作没有。删除组时在同一操作里删除它的 `aicc_routing_binding` 行。

### 4.6 Table: `aicc_routing_binding`

Description：用户把自己的策略组绑定到 App（Agent 的 `app_id` 就是 agent_id）。会话绑定不在这里，由 lib_opendan 保存。

| Column | Type | Nullable | Description |
|---|---|---:|---|
| `owner` | TEXT | NO | `user:<user_id>` |
| `scope` | TEXT | NO | `app:<app_id>` |
| `group_id` | TEXT | NO | 属于同一 `owner` 的策略组 |
| `updated_at_ms` | BIGINT | NO | Unix ms |

Primary key：`(owner, scope)`，一个用户给一个 App 只绑一个组；`routing.bindings.set` 对已有的绑定直接改写。

Lifecycle：调用时按 `(caller.user_id, caller.app_id)` 查；绑定指向的组不存在时忽略。

### 4.7 Table: `aicc_artifact_scope`

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

### 4.8 Table: `aicc_artifact_url_source`

Description：AICC 对 Provider 输出中 `ResourceRef::Url` 建立的来源和授权索引。普通调用方 URL 不写入本表，AICC 不为未登记 URL 提供下载代理。

| Column | Type | Nullable | Description |
|---|---|---:|---|
| `url_hash` | TEXT PK | NO | 完整 URL 的 SHA-256，作为定长查询键 |
| `url` | TEXT | NO | Provider 返回的精确 URL；查询命中后必须再次逐字比较以防 hash 冲突 |
| `provider_instance_name` | TEXT | NO | 产生 URL 的 ProviderInstance |
| `protocol_adapter_id` | TEXT | NO | 产生 URL 时实际选中的客户端；读取时它必须仍在实例的客户端集合内 |
| `origin_provider` | TEXT | NO | 产出内容的模型原厂 |
| `artifact_id` | TEXT | YES | Adapter 显式绑定的 Provider artifact id；缺失时为空 |
| `content_digest` | TEXT | YES | 完整读取 URL 内容后补全的 `sha256:<hex>`；登记时为空 |
| `expires_at_ms` | BIGINT | YES | Provider 明示的 artifact 绝对过期时间（Unix ms）；未声明时为空 |
| `tenant_id` | TEXT | NO | 创建 tenant，读取授权的强制边界 |
| `user_id` | TEXT | NO | 创建用户，用于审计 |
| `caller_app_id` | TEXT | NO | 创建 app；当前无法获得时为空串 |
| `request_id` | TEXT | NO | 产生 artifact 的请求 ID |
| `created_at_ms` | BIGINT | NO | Unix ms |

Indexes：`idx_aicc_artifact_url_source_tenant(tenant_id, created_at_ms)`、`idx_aicc_artifact_url_source_provider(provider_instance_name, created_at_ms)`。

Constraints：复用输入时必须先校验 tenant/user；同 ProviderInstance 或结果 URL 与来源实例当前 base URL 不同 origin 时可直接传递，跨实例同 origin 时通过原 ProviderInstance 下载为字节。来源实例不可用时不得按公网 URL 猜测处理。需要下载已登记 URL 时，初次物化、URL fallback 和调用方下载接口均复用同一个 artifact reader，由来源 Adapter 决定鉴权及跨域下载规则，外围不再根据 origin 选择另一条下载路径。打开 reader 时必须同时满足 URL 已登记、精确 URL 相同、当前 tenant/user 相同、ProviderInstance 仍存在且 Adapter 未改变；调用方传入 `artifact_id` 时还必须与登记值一致。`expires_at_ms` 非空且小于等于当前时间时记录视为失效并删除，不再尝试下载。内容流完整读到 EOF 后以实际下载字节计算摘要并补全；失败或提前停止保持为空。URL 可能包含 Provider 签名参数，禁止写入日志、错误消息或审计 data。

### 4.9 Table: `aicc_provider_artifact`

Description：把已交给 Provider 的内容映射到该 ProviderInstance 与模型原厂内的 opaque artifact ID，使后续请求可以直接传 ID、免去再次传输内容。**这是纯缓存**：整表清空后功能仍然正确，只是每次调用都要重新传输内容。

两种键共用一张表，互斥且恰好一列非空：

- `content_digest` 键标识**内容本身**。同一份字节换文件名、换 MIME 仍是同一行；代价是先读完内容才能计算摘要。
- `obj_id` 键标识**对象身份**。它随 `ResourceRef::NamedObject` 免费到手，命中过程不需要读取任何字节；代价是对象 ID 把 `name` 与 attributes 一起算进哈希，同一份字节换个名字就是另一个对象 ID。

| Column | Type | Nullable | Description |
|---|---|---:|---|
| `content_digest` | TEXT | NO | `sha256:<hex>` 内容摘要；obj_id 键的行此处为空串 |
| `obj_id` | TEXT | NO | `ResourceRef::NamedObject` 的对象 ID；content_digest 键的行此处为空串 |
| `provider_instance_name` | TEXT | NO | 具体账号/入口 namespace；Provider ID 不得跨实例复用 |
| `origin_provider` | TEXT | NO | 模型原厂 namespace；聚合 Provider 内不得跨原厂复用 |
| `tenant_id` | TEXT | NO | 缓存归属 tenant；不得跨 tenant 复用 |
| `artifact_id` | TEXT | NO | Provider 返回且由 Adapter 明确绑定的 opaque ID |
| `expires_at_ms` | BIGINT | YES | Provider 明示的 artifact 绝对过期时间（Unix ms）；未声明时为空 |
| `created_at_ms` | BIGINT | NO | Unix ms |

Primary key：`(content_digest, obj_id, provider_instance_name, origin_provider, tenant_id)`。

Constraint：`CHECK ((content_digest='') <> (obj_id=''))` —— 恰好一种键非空；两种键都空或都非空的行会被拒绝，不会写出无法命中的记录。

Index：`idx_aicc_provider_artifact_provider(provider_instance_name, origin_provider, created_at_ms)`。

Constraints：查找必须提供完整主键（键 + ProviderInstance + `origin_provider` + tenant）；ProviderInstance、`origin_provider` 或 tenant 任一不同都不得复用 ID。`expires_at_ms` 非空且小于等于查询时间时必须在返回前删除并按未命中处理；为空表示 Provider 未声明期限，不代表永久有效。Provider 使用该 ID 返回明确的 HTTP 404 时，按完整主键和 artifact ID 条件删除，避免并发产生的新 ID 被旧失败误删。摘要是对内容本身的原始字节直接计算的 SHA-256，不得加入盐、MIME、文件名或身份字段；Base64 必须先解码。URL artifact 登记时摘要为空，只有内容流完整读到 EOF 后才补全 URL 来源记录、**并以 content_digest 键写入本表**（沿用原 ID 的创建时间和过期时间）。相同 namespace 对同一内容返回新 ID 时以最新成功结果覆盖。只有 Adapter 明确声明的 ID 才能登记，禁止从 URL、artifact name 或模型名猜测；native task adapter 可以显式声明 task ID 同时也是 artifact ID。

两种键的取用顺序：对象形态（`ResourceRef::NamedObject`）的入参优先查 `obj_id` 键；未命中时**不回退** `content_digest` 键——回退等于读完整份内容，正是这条路径要避免的成本——直接改用对象 URL。字节形态的入参本来就有内容，直接查 `content_digest` 键。

### 4.10 `aicc_usage_event` 的路由列

usage event 的完整表结构见 [aicc_usage_log_db_requirements.md](aicc_usage_log_db_requirements.md)。路由相关的列：

| Column | Type | Nullable | Description |
|---|---|---:|---|
| `fallback_from` | TEXT | YES | 这次用量由用途的“不可用时”策略切换而来时，被替换的原精确模型；`usage.query` 的 `fallback_only` 按它非空过滤。用量只记在实际执行的模型上 |

## 5. Schema Version

`aicc_schema_meta.schema_version` 当前为 `4`，对应 `storage/schema.sql`（`SCHEMA`）与 `storage/policy.sql`（`POLICY_SCHEMA`）组成的完整表集：route trace 过滤列 `fallback_applied` / `warning_count`、usage 归属列 `operator_id` / `attribution_json`、Credit 列 `credit_micro` / `credit_status`、路由列 `fallback_from`，用户策略与 Credit 各表，以及用途表的 `aicc_routing_layer` / `aicc_routing_group` / `aicc_routing_binding`。

打开时：没有任何 `aicc_*` 表的空 RDB 在一个事务内建齐完整表集并写入版本 `4`；记录版本 `4` 的 RDB 在同一事务里补建缺少的用途表三张表和 `aicc_usage_event.fallback_from` 列（它们是版本 4 之内新增的，版本号不变）；记录版本 `3` 的 RDB 升级一次（删除旧的 token 额度与可见性表，保留 usage 历史，补齐版本 4 的列和表）。其它旧版本、未来版本，以及存在 `aicc_*` 表却缺少版本记录的 RDB 都报 unsupported schema，不补列、不回填、不改写版本、不删表。Beta 2.2 之前的开发期 RDB 不提供升级路径，需要重置时使用明确的全新安装。`finance_snapshot_json` 仍是权威数据，普通列是写入时生成且可直接信任的查询投影，读取时不反向解析 JSON，也不做双份校验。

`usage.query` 的时间、身份、模型、Provider、method 等过滤条件、事件游标和页大小均进入 SQL；summary、group 和 time bucket 由数据库基于 typed projection 聚合，只将聚合行和请求页返回进程，不再先加载全部命中事件。

各业务表的 `*_json` 是完整权威 payload，普通 typed columns 是同一次写入生成、可直接信任的查询投影。读取列表、过滤、聚合和分页可以只读投影列，无需加载 JSON 或逐行复核两者相等；需要重建完整对象时才读取 JSON。

## 6. Upgrade Compatibility Strategy

当前为 Beta 2.2 breaking change：

| 数据项 | 策略 |
|---|---|
| `aicc_execution_record` | No-compat；进入正式发布后必须 migration，不能 rebuild 运行中任务 |
| `aicc_route_trace_event` | No-compat；将来可按 retention 清理，不可伪造重建 |
| `aicc_routing_layer` / `aicc_routing_group` / `aicc_routing_binding` | 用户配置，不可从其它数据重建；进入正式发布后必须 migration |
| `aicc_artifact_scope` | No-compat；不得从对象内容推测 tenant 重建 |
| `aicc_artifact_url_source` | No-compat；不得根据 URL host 猜测 Provider 或 owner 重建 |
| `aicc_provider_artifact` | No-compat；纯缓存，清空只损失加速；只有再次获得 Provider 明确返回的 ID 后才能重建 |

schema 版本或结构不受支持时必须 error-and-stop，不得带着部分新 schema 继续接受请求。

## 7. Extensibility Rules

- Frozen：各表主键、tenant 隔离语义、idempotency fingerprint 语义、artifact owner 首次写入语义、URL 必须精确匹配已登记来源的语义。
- Extensible：`record_json`、`route_trace_json` 可增加有默认值的脱敏字段；关键查询/授权字段必须升格为 typed column，不能只藏在 JSON。
- 禁止用通用 `extra` 改变租户授权、执行恢复或 session 路由语义。
- 新列必须有明确缺省值或 migration；不允许重解释旧列。

## 8. Query Patterns

| 查询 | 索引 | 频率/边界 |
|---|---|---|
| 按 tenant/method/idempotency key 查执行 | execution PK | 每次带 key 请求，高 |
| 按 task ID 恢复/取消 | execution unique task index | 任务观察，高 |
| 启动恢复未终结任务 | execution state index | 每次启动，应并发 poll，不得被单个旧任务阻塞 |
| 读调用者的用户层 | routing layer PK | 每次路由，高 |
| 按 `(owner, app:<app_id>)` 查 App 绑定，再按 `group_id` 读组 | binding PK、group PK | 每次带 app_id 的路由，高 |
| 按 `group_id` 读请求的 `route_group` 并核对 owner | group PK | 每次带 `route_group` 的路由，高 |
| 列出用户自己的组和绑定 | group owner index、binding PK 前缀 | AI Center 用途页 / 策略组页，低 |
| 按 obj_id 校验 artifact tenant | artifact PK | 每次读 AICC 生成 Named Object，高 |
| 按 tenant/time 列出 artifact | artifact tenant index | 管理/维护，低 |
| 按 URL 打开 Provider artifact | artifact URL hash PK，再精确比较 URL | 每次读取 URL artifact，高 |
| 按 tenant/provider/time 维护 URL 来源 | artifact URL tenant/provider index | retention 与诊断，低 |
| 按内容摘要、ProviderInstance、origin provider 查询未过期 opaque ID | provider artifact PK | 媒体输入物化，高；三元组必须完整，到期记录同步删除 |
| 按 trace/task/tenant/time 查 route trace | 对应组合索引 | 诊断，中 |
| 按 expiry 清理幂等记录 | execution expiry index | 维护，低 |
