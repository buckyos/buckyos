# Provider Driver Metadata 云更新详细设计

状态：Draft  
适用版本：beta 2.2 breaking change，不做向前兼容  
相关文档：

- `doc/aicc/driver_metadata_schema.md`
- `doc/aicc/aicc-models-mgr.md`
- `doc/aicc/aicc_router.md`
- `doc/aicc/aicc_provider修改.md`

## 1. Overview

服务名：

- `provider-metadata-tech-service`：技术参数云服务，下文简称 A 服务。
- `provider-metadata-ops-service`：运营参数云服务，下文简称 B 服务。

说明：A 服务 / B 服务只是本文档为了描述架构职责而使用的简称，不是产品 UI 文案。WebUI 不得直接展示“A 服务”“B 服务”“A/B”等字样，应使用“技术参数”“运营参数”“技术源”“同步源”“发布源 revision”“运营 revision”等名称。

本设计定义 provider-driver metadata 的云端维护、发布、客户端拉取和本地增量应用机制。A 服务负责相对稳定的技术事实与 key 字段，B 服务负责相对高频的运营参数，并可从 A 服务拉取技术参数后合并成客户端可直接消费的完整配置。客户端最终接收的发布格式是按 `provider_driver` 组织的 driver metadata document，字段保持为 `schema_version`、`provider_driver`、`name`、`base_url`、`revision`、`models`、`patterns`、`defaults`、`variants`、`version_rules`、`signature`，供 AICC metadata resolver 生成 `ProviderInventory`、`ModelMetadata`、`logical_mounts`、`capabilities`、`pricing` 等运行时结构。

### 1.1 Client Publish View

本文中“发布”指客户端最终拉取并消费的内容，不指云端 RDB、对象存储、编辑草稿或审计日志的存储形态。云端可以使用 provider、model_param_rules、metadata_variants、metadata_version_rules、model_nicks、ops_overlays 等表格化数据维护草稿和审计事实；发布预览、发布缓存和客户端下发内容必须由这些数据物化为 `doc/aicc/driver_metadata_schema.md` 定义的 driver metadata document。

客户端下发单元按 `provider_driver` 组织，等价于 `$BUCKYOS_ROOT/etc/aicc/driver_metadata/remote_cache/<driver>.json` 可接收的内容。每个 document 只包含：

- `schema_version`
- `provider_driver`
- `name`
- `base_url`
- `revision`
- `models`
- `patterns`
- `defaults`
- `variants`
- `version_rules`
- `signature`

`name` 和 `base_url` 是 driver metadata 下发字段。`base_url` 用于区分兼容 provider 和 endpoint 匹配，缺省时客户端按 `provider_driver` 取默认 endpoint；`name` 用于 provider 推荐和类型展示，不是用户创建的 provider instance 展示名。

`provider_key`、`provider_kind`、`protocol_family`、源对象引用、`model_nicks`、`logical_directories`、`dictionaries`、`ops_overlays`、`recommendation_level`、`display_priority`、`routing_weight`、`ops_note` 等都是管理、编辑、校验或运营合并输入，不是 driver metadata 下发字段。它们可以影响是否下发、下发到哪个 selector、以及合法 driver metadata 字段的取值，但不能原样出现在客户端 driver metadata document 中。

## 2. Data Classification

| 数据项 | 归属 | 分类 | 说明 |
|---|---|---|---|
| Provider 技术主数据 | A | Durable | `provider_driver`、`name`、`base_url`、provider key、协议族、默认 endpoint 等。 |
| Model 参数匹配规则 | A | Durable | 通过 `exact` / `pattern` / `default` 匹配原始 `model.id`，并赋予 `api_types`、`capabilities`、上下文长度、默认 logical mounts、技术 tags 等参数。 |
| Variants/version_rules | A | Durable | driver metadata resolver 的附加规则输入。 |
| Provider 源对象引用 | A | Durable | provider 白名单引用的 models、patterns、defaults、variants、version rules 及其 source key；排除模型最终表达为 exact/pattern `model_param_rules.exclude=true`。 |
| 运营 overlay | B | Durable | provider/model 禁用、推荐权重、运营价格覆盖、展示优先级、灰度策略。 |
| 发布 revision | A/B | Durable | 每次有效发布递增，客户端据此判断是否更新。 |
| 编辑会话快照 | A/B | Durable | 进入编辑模式时保存，提交时用于 diff、影响范围分析和审计。 |
| 变更日志 | A/B | Durable | 记录操作者、导入文档、diff、审批结果、发布 revision。 |
| 导入更新计划 | A/B | Durable | 人或 AI 生成的可读文本，导入后成为待提交变更。 |
| 聚合发布缓存 | A/B | Disposable | 从 RDB 物化出的 driver metadata document，可随时重建。 |
| A 服务远端查询缓存 | B | Disposable | B 服务拉取 A 服务结果后的短期缓存，可在服务启动或发布后重建。 |
| API HTTP cache metadata | A/B | Disposable | ETag、Last-Modified、压缩缓存。 |
| WebUI 检索索引 | A/B | Disposable | 搜索加速索引，可由 durable data 重建。 |

## 3. Storage Strategy

结构化持久数据必须使用平台 RDB 实例，不绑定 sqlite、PostgreSQL 等具体后端。发布 JSON、预览结果和搜索索引是可丢弃缓存，由 RDB 中的实体和规则重建。

对象型数据仅用于保存导入文本、diff 报告、测试用例草案等较大的审计附件；它们通过 object id 被 RDB 记录引用，不作为核心查询模型。

## 4. Schema Definitions

### Table: metadata_meta

| Column | Type | Nullable | Default | Description |
|---|---|---|---|---|
| key | TEXT PK | NO | | 元数据 key。 |
| value | TEXT | NO | | 元数据 value。 |
| updated_at | INTEGER | NO | | Unix timestamp ms。 |

约束：

- `schema_version` 必须存在。
- `published_revision` 必须存在。

### Table: providers

| Column | Type | Nullable | Default | Description |
|---|---|---|---|---|
| provider_key | TEXT PK | NO | | 服务端自动编号生成的稳定 provider key；创建后不可修改。 |
| provider_driver | TEXT | NO | | AICC driver id，如 `openai`、`claude`、`google-gemini`、`fal`。 |
| name | TEXT | YES | | 主流展示名，如 `OpenRouter`；不是用户创建的 provider instance 展示名。 |
| base_url | TEXT | YES | | provider endpoint 匹配 key；为空时客户端按 `provider_driver` 取默认 endpoint。 |
| provider_kind | TEXT | NO | `origin` | `origin`（原厂）/ `aggregator`（聚合多个上游 inventory）。协议兼容性不属于 kind。 |
| protocol_family | TEXT | YES | | 客户端调用该 endpoint 时使用的 wire protocol，如 `openai-compatible`、`anthropic`、`google`；多个 provider 可复用同一协议族。 |
| enabled | INTEGER | NO | `1` | A 服务技术层是否发布。 |
| owner_service | TEXT | NO | `A` | 字段主责服务，固定为 A。 |
| extra | TEXT | YES | | JSON 扩展字段。 |
| created_at | INTEGER | NO | | Unix timestamp ms。 |
| updated_at | INTEGER | NO | | Unix timestamp ms。 |

Indexes:

- `idx_providers_driver` ON `providers(provider_driver)`。
- `idx_providers_base_url` ON `providers(base_url)`。
- `idx_providers_name` ON `providers(name)`。

### Table: model_param_rules

| Column | Type | Nullable | Default | Description |
|---|---|---|---|---|
| rule_key | TEXT PK | NO | | 稳定规则 id。 |
| provider_key | TEXT | YES | | 为空表示全局/原厂规则；非空表示 provider 专属规则。 |
| source_rule_key | TEXT | YES | | 引用的全局/原厂规则；为空表示完全自定义。 |
| match_type | TEXT | NO | | `exact` / `pattern` / `default`。 |
| original_provider | TEXT | YES | | 限定原厂；`default` 可为空。 |
| model_id_selector | TEXT | YES | | `exact` 时为原始 `model.id`；`pattern` 时为 wildcard pattern；`default` 为空。 |
| priority | INTEGER | YES | | `pattern` 匹配顺序，越小越早匹配；`exact/default` 为空。 |
| model_driver | TEXT | YES | | 可覆盖 `provider_driver`，用于聚合商上游归属。 |
| api_types | TEXT | NO | `[]` | JSON array。 |
| logical_mounts | TEXT | NO | `[]` | JSON array，支持当前 `model.logical_mounts` 通配符语法。 |
| capabilities | TEXT | NO | `{}` | JSON object，对应 `ModelCapabilities` patch。 |
| attributes | TEXT | YES | | JSON object，对应 `ModelAttributes`。 |
| context_limits | TEXT | YES | | JSON object，如 `max_context_tokens`、`max_output_tokens`。 |
| pricing | TEXT | YES | | JSON object，参考价格；动态 provider 价格优先。 |
| exclude | INTEGER | NO | `0` | 技术层发布排除标记，仅适用于 `exact` / `pattern`。为 true 时该规则发布为排除规则，其它参数字段当前不参与发布语义但保留以便恢复。 |
| extra | TEXT | YES | | JSON 扩展字段。 |
| enabled | INTEGER | NO | `1` | 是否启用。 |
| created_at | INTEGER | NO | | Unix timestamp ms。 |
| updated_at | INTEGER | NO | | Unix timestamp ms。 |

Indexes:

- `idx_model_param_rules_exact` ON `model_param_rules(provider_key, original_provider, model_id_selector, match_type)`。
- `idx_model_param_rules_pattern` ON `model_param_rules(provider_key, priority)`。
- `idx_model_param_rules_original_provider` ON `model_param_rules(original_provider)`。

约束：

- `exact`、`pattern`、`default` 本质上都是“对模型分类并赋参数”的规则，必须保存在同一张表里，通过 `match_type` 区分。
- 匹配顺序固定为：先匹配 `exact`，再按 `priority` 匹配 `pattern`，最后使用 `default`。一个模型只允许命中一条最终参数规则。
- `exact` 必须设置 `original_provider + model_id_selector`，并且 `model_id_selector` 是原始 `model.id`。
- `pattern` 必须设置 `model_id_selector`，并按 `priority` 稳定排序；发布前必须按目标 provider 的 `model_nicks` 把 selector 改写为客户端可见的 published model id pattern。
- `default` 的 `model_id_selector` 和 `priority` 必须为空；每个 provider scope 最多一条启用的 default，作为 exact 和 pattern 全部失败后的 fallback。
- `exclude=true` 只允许用于 exact/pattern。default 不表达排除语义。
- 统一存储只影响后端 RDB 结构，不改变发布 JSON 结构。发布 JSON 必须把 `match_type=exact` 物化到 `models[]`，把 `match_type=pattern` 按 `priority` 稳定排序后物化到 `patterns[]`，把 `match_type=default` 物化到 `defaults`。
- WebUI 可支持从任意 `match_type` 的规则复制创建另一种 `match_type` 的新规则，以复用参数字段；这不是原地修改 `match_type`。新规则必须重新满足目标 `match_type` 的 selector、priority 和唯一性约束。
- 复制创建时如果来源 `match_type` 和目标 `match_type` 不一致，WebUI 必须提示用户确认类型变更，展示来源类型、目标类型以及会按目标类型新增、改写或清空的字段；最终 `match_type` 以用户选择的目标类型为准。
- provider 专属规则可以引用全局/原厂规则后通过字段覆盖变成专属配置，也可以完全自定义。
- provider 专属规则只影响参数匹配，不自动限定 provider 的源模型候选集合。

### Table: model_nicks

| Column | Type | Nullable | Default | Description |
|---|---|---|---|---|
| nick_key | TEXT PK | NO | | 服务端自动编号生成的稳定 nick id；创建后不可修改。 |
| provider_key | TEXT | NO | | 目标 provider。 |
| original_provider | TEXT | YES | | 原厂限制。 |
| model_id | TEXT | NO | | 原始模型 id 或 pattern。 |
| nick | TEXT | NO | | 下发给客户端的模型 id。 |
| selector_type | TEXT | NO | `exact` | `exact` / `pattern`。 |
| priority | INTEGER | NO | | 批量 nick 顺序。 |
| created_at | INTEGER | NO | | Unix timestamp ms。 |
| updated_at | INTEGER | NO | | Unix timestamp ms。 |

Indexes:

- `idx_model_nicks_provider` ON `model_nicks(provider_key, priority)`。

Constraints:

- `nick` 属于 key 性字段，由 A 服务管理。
- 发布时 `models[].id` 使用 nick 后的 id；原始 id 只保存在 diff、trace、审计和编辑态记录中，不作为客户端 driver metadata 字段下发。
- Nick 是复用源对象时的发布期中间映射，不是生成一份已改名的模型/variant/version rule 备份。规则可有多条，支持 `origin-prefix`（如 `* -> openai/{model}`）和 exact/pattern mapping（如 `gpt-image-1 -> gpt-image`、`gpt-* -> xpt-*`）。
- Nick 重写必须作用于客户端会看到的模型选择字段：`models[].id`、`patterns[].pattern`、variants 的 `model_id_selector`，以及 version rules 的 `content.model_pattern`。default 规则没有下发 selector，只作为参数兜底规则参与预览。variant 未显式声明模型 selector 时，其默认 selector 为 `*`，该 `*` 同样可以被重写。

### Table: metadata_variants

| Column | Type | Nullable | Default | Description |
|---|---|---|---|---|
| variant_key | TEXT PK | NO | | 服务端自动编号生成的稳定 variant id；创建后不可修改。 |
| provider_key | TEXT | YES | | 为空表示全局 variant；非空表示 provider 专属 variant。 |
| source_variant_key | TEXT | YES | | 引用的全局/原厂 variant；为空表示完全自定义。 |
| selector_type | TEXT | NO | `exact` | `exact` / `pattern`。 |
| original_provider | TEXT | YES | | 限定原厂。 |
| model_id_selector | TEXT | YES | `*` | 原始 `model.id` 或 wildcard pattern；为空时按 `*` 处理；发布时按 model nick 规则重写。 |
| priority | INTEGER | NO | `0` | 同一 provider 下的匹配顺序。 |
| nick | TEXT | YES | | 下发给客户端的 variant id；为空时使用 `variant_key`。 |
| content | TEXT | NO | | variant JSON content 或对 `source_variant_key` 的 patch。 |
| enabled | INTEGER | NO | `1` | 是否启用。 |
| created_at | INTEGER | NO | | Unix timestamp ms。 |
| updated_at | INTEGER | NO | | Unix timestamp ms。 |

Indexes:

- `idx_metadata_variants_provider_priority` ON `metadata_variants(provider_key, priority)`。

### Table: metadata_version_rules

| Column | Type | Nullable | Default | Description |
|---|---|---|---|---|
| version_rule_key | TEXT PK | NO | | 服务端自动编号生成的稳定 version rule id；创建后不可修改。 |
| provider_key | TEXT | YES | | 为空表示全局 version rule；非空表示 provider 专属 version rule。 |
| source_version_rule_key | TEXT | YES | | 引用的全局/原厂 version rule；为空表示完全自定义。 |
| selector_type | TEXT | NO | `pattern` | `exact` / `pattern`。 |
| original_provider | TEXT | YES | | 限定原厂。 |
| model_id_selector | TEXT | NO | | 便于列表筛选和命中预览的原始 `model.id` 或 wildcard pattern；保存时与 `content.model_pattern` 保持一致。发布重写以 `content.model_pattern` 为准。 |
| priority | INTEGER | NO | `0` | 同一 provider 下的匹配顺序。 |
| nick | TEXT | YES | | 下发给客户端的 rule id；为空时使用 `version_rule_key`。 |
| content | TEXT | NO | | version rule JSON content 或对 `source_version_rule_key` 的 patch。 |
| enabled | INTEGER | NO | `1` | 是否启用。 |
| created_at | INTEGER | NO | | Unix timestamp ms。 |
| updated_at | INTEGER | NO | | Unix timestamp ms。 |

Indexes:

- `idx_metadata_version_rules_provider_priority` ON `metadata_version_rules(provider_key, priority)`。

约束：

- `variants/version_rules` 与 `model_param_rules` 一样是 A 服务管理的一等对象；每个 provider 可为每一种类型配置多条记录。
- `variants` 使用 `metadata_variants` 表管理；`version_rules` 使用 `metadata_version_rules` 表管理。
- 这些对象通常引用全局/原厂配置；provider 专属记录默认通过 source key + content patch 继承原厂配置，只有修改字段后才成为专属配置。
- `metadata_variants.model_id_selector` 匹配原始 `model.id`；为空时按 `*` 处理。`metadata_version_rules.content.model_pattern` 是 version rule 的模型通配字段，`model_id_selector` 只是列表筛选和命中预览用的镜像字段。发布给客户端前，variants 的 `model_id_selector` 和 version rules 的 `content.model_pattern` 都必须按该 provider 的 `model_nicks` 规则改写为 nick 后的 selector；无法重写或重写后冲突时阻止发布。
- `variants` 必须显式支持 `selector_type/original_provider/model_id_selector`，不能只靠 variant `name` 隐式匹配 base model。

### Table: ops_overlays

| Column | Type | Nullable | Default | Description |
|---|---|---|---|---|
| overlay_key | TEXT PK | NO | | 稳定 overlay id。 |
| target_type | TEXT | NO | | `provider` / `model_param_rule` / `variants` / `version_rules`。 |
| target_key | TEXT | NO | | A 服务 key 或发布 key。 |
| disabled | INTEGER | NO | `0` | B 服务是否禁用该对象。 |
| ops_patch | TEXT | NO | `{}` | B 服务管理的运营字段 patch。 |
| created_at | INTEGER | NO | | Unix timestamp ms。 |
| updated_at | INTEGER | NO | | Unix timestamp ms。 |

Indexes:

- `idx_ops_overlays_target` ON `ops_overlays(target_type, target_key)`。

Constraints:

- `ops_patch` 不得包含 A 服务管理字段；发现污染字段时发布阶段丢弃该字段，不丢弃整条记录。
- B 服务首版允许写入的运营字段最小集合固定为 `disabled`、`pricing override`、`routing weight`、`recommendation level`、`display priority`；后续新增运营字段仍写入 `ops_patch`，但必须保持 A/B 字段所有权校验。

### Table: edit_sessions

| Column | Type | Nullable | Default | Description |
|---|---|---|---|---|
| session_id | TEXT PK | NO | | 编辑会话 id。 |
| service_role | TEXT | NO | | `A` / `B`。 |
| operator_id | TEXT | NO | | 操作者账号。 |
| base_revision | TEXT | NO | | 开始编辑时的发布 revision。 |
| snapshot_object_id | TEXT | NO | | 快照对象 id。 |
| status | TEXT | NO | `editing` | `editing` / `previewed` / `approved` / `published` / `discarded`。 |
| created_at | INTEGER | NO | | Unix timestamp ms。 |
| updated_at | INTEGER | NO | | Unix timestamp ms。 |

说明：

- `editing` 状态的 `edit_session` 是可持久保存的草稿。一次 metadata 更新可能持续较长时间，管理员可以保存草稿后退出。
- 管理员下次登录时，管理端必须查询其未完成的 `editing` / `previewed` edit session，并在工作区提示可继续处理的草稿。
- 草稿列表至少展示 service role、base revision、updated_at、操作者和变更摘要；管理员可以逐个选择继续编辑、进入发布预览或放弃。
- 如果草稿的 `base_revision` 已落后于当前 `published_revision`，不得直接发布，必须重新生成 diff、影响范围和发布预览。

### Table: change_logs

| Column | Type | Nullable | Default | Description |
|---|---|---|---|---|
| change_id | TEXT PK | NO | | 变更 id。 |
| service_role | TEXT | NO | | `A` / `B`。 |
| operator_id | TEXT | NO | | 操作者账号。 |
| from_revision | TEXT | NO | | 发布前 revision。 |
| to_revision | TEXT | NO | | 发布后 revision。 |
| source_revision | TEXT | YES | | B 服务发布时使用的 A revision；A 服务发布时为空。 |
| source_stale | INTEGER | NO | `0` | B 服务发布时使用的 A 缓存是否处于 stale 状态。 |
| summary | TEXT | NO | | 人类可读摘要。 |
| diff_object_id | TEXT | NO | | diff 报告对象 id。 |
| import_object_id | TEXT | YES | | 导入更新计划对象 id。 |
| test_plan_object_id | TEXT | YES | | 生成的测试建议对象 id。 |
| created_at | INTEGER | NO | | Unix timestamp ms。 |

Object Type: `metadata_update_plan`

- Description：人类或 AI 生成的更新计划文本。
- Naming Convention：`metadata-update-plan/<change_id>/<file_name>`。
- Content Format：UTF-8 YAML 或 Markdown 包裹 YAML。

Object Type: `metadata_diff_report`

- Description：提交前后差异、影响范围、被污染字段、生成测试建议。
- Naming Convention：`metadata-diff-report/<change_id>.json`。
- Content Format：JSON。

## 5. Schema Version

初始 `schema_version = 1`，存储在 `metadata_meta` 表：

```text
key = schema_version
value = 1
```

任意 RDB 表结构、发布 JSON 语义或导入文本 schema 发生不兼容变化时递增 schema version。provider-driver metadata 自身的业务 revision 与 schema version 分离：schema version 表示存储格式，published revision 表示配置内容版本。

## 6. Upgrade Compatibility Strategy

当前版本处于 beta 2.2 breaking change 阶段，云服务可按 No-compat 处理首次落地数据；正式发布后采用以下策略：

| 数据项 | 策略 | 说明 |
|---|---|---|
| `metadata_meta` | Additive-only | 新 key 可增加，已有 key 语义冻结。 |
| `providers` | Migration | key 字段语义冻结，新增字段走 migration。 |
| `model_param_rules` | Migration | `exact` / `pattern` / `default` 匹配语义、参数字段 schema 或优先级规则改动需要迁移。 |
| `model_nicks` | Migration | nick 会影响客户端 key，修改语义必须显式迁移。 |
| `metadata_variants` | Migration | variants schema 改动需要迁移。 |
| `metadata_version_rules` | Migration | version_rules schema 改动需要迁移。 |
| `ops_overlays` | Additive-only | 新运营字段写入 `ops_patch`。 |
| `edit_sessions` | Persistent draft | 正常版本内未完成编辑会话必须可恢复；大版本升级时可要求重新预览或重新打开。 |
| `change_logs` | Additive-only | 审计日志只追加，不修改历史。 |

Migration 在服务启动时执行。失败时服务进入只读模式，继续提供上一版已发布缓存；管理端写入暂停，避免生成不可审计的发布。

## 7. Extensibility Rules

冻结字段：

- Provider key：`provider_key`、`provider_driver`、`base_url`、`name`。
- Model key：`model_id`、`original_provider`、`provider_key`。
- Nick key：`nick`、`model_id`、`provider_key`。
- Model param rule match type：`exact`、`pattern`、`default` 由 `match_type` 决定，创建后不允许原地转换。
- Revision：所有已发布 revision 不可复用。

字段编辑原则：

- 原则上，拥有 `metadata.update` 授权的管理员可以编辑对应云服务负责的所有属性字段；字段所有权仍按第 10 章执行，B 服务不得通过 overlay 修改 A-only 字段。
- key 性质字段通常只在构建或首次导入后确定，发布后不建议变更，包括 `provider.name`、`provider.base_url`、`model.id`、`original_provider`、`nick`，以及对应稳定 key 字段。
- WebUI 对 key 性质字段必须设置防御性障碍：即使页面已经进入编辑模式，这些字段仍保持预览/只读状态。`provider_key`、`nick_key`、`variant_key`、`version_rule_key` 由服务端自动编号，不提供字段级修改。
- 提交包含 key 性质字段变更的更新时，必须在 diff 和发布确认中单独列出影响范围，并展示警告性提示，包括客户端三方合并、provider endpoint 匹配、model id/nick 重写、逻辑目录引用和历史审计 trace 的影响。

可扩展字段：

- `extra`、`attributes`、`capabilities`、`pricing`、`ops_patch` 可增加子字段。
- `model_param_rules` 的参数字段以及 `metadata_variants.content`、`metadata_version_rules.content` 可按各自类型增加字段，但必须更新对应 schema 文档、WebUI 表单和发布合成校验。
- provider 源对象引用的类型集合固定为 models、patterns、defaults、variants、version rules；新增可引用对象类型时必须同时更新反向引用诊断、发布合成和 WebUI 选择器。

清理规则：

- provider 专属 `model_param_rules` 如果和对应全局/原厂规则等价，提交前应自动删除该专属记录，直接引用全局/原厂规则。
- B 服务 overlay 里出现 A 服务字段时，发布阶段丢弃污染字段并记录 warning，不丢弃整个 provider/model。

## 8. Query Patterns

| 查询 | 频率 | 索引 |
|---|---|---|
| 按 revision 获取发布 JSON | 高 | `metadata_meta.published_revision` + 发布缓存。 |
| 按 provider_driver 列 provider | 高 | `idx_providers_driver`。 |
| 按 base_url/name 匹配 provider | 高 | `idx_providers_base_url`、`idx_providers_name`。 |
| 按 provider 列模型参数规则 | 高 | `idx_model_param_rules_exact`、`idx_model_param_rules_pattern`。 |
| 按原厂列全局模型参数规则 | 高 | `idx_model_param_rules_original_provider`。 |
| 合成某 provider 的最终 models | 中 | provider 源对象引用索引、`idx_model_param_rules_exact`、`idx_model_param_rules_pattern`、`idx_model_nicks_provider`。 |
| 按 pattern 顺序匹配 model | 中 | `idx_model_param_rules_pattern`。 |
| B 合并 A 发布快照 | 中 | `idx_ops_overlays_target`。 |
| WebUI 搜索 provider/model | 高 | RDB 索引 + 可丢弃搜索索引。 |
| 变更审计查询 | 低 | `change_logs.created_at` 可后续加索引。 |

发布 JSON 构建允许全量扫描，因为配置更新频率低。客户端 GET 路径必须读发布缓存，不应实时 JOIN 生成。

## 9. 架构与职责边界

### 9.1 A 服务职责

A 服务是技术事实源，负责：

- 汇总各 provider-driver 的技术 metadata。
- 管理 provider key、`provider_driver`、`base_url`、`name` 等 key 字段。
- 管理全局/原厂和 provider 专属 `model_param_rules`，包括 exact、pattern、default 三类模型参数匹配规则。
- 管理 variants、version_rules。
- 管理 provider 源模型候选集合 include 规则，并提供批量 exclude 操作生成 exact/pattern `model_param_rules.exclude=true`。
- 管理 model nick 规则。
- 生成 A 服务自身的发布 JSON。

A 服务只开放读取发布配置的公开 GET API。更新配置需要登录账号和 `metadata.update` 授权。

### 9.2 B 服务职责

B 服务是运营 overlay 源，负责：

- 保存 A 服务查询 URL。
- 定期或按需拉取 A 服务发布 JSON。
- 在本地 overlay 中管理运营参数。
- 禁用特定 provider/model/model_param_rule/variant/version_rule。
- 合并 A 技术参数与 B 运营参数，生成客户端完整发布 JSON。
- 生成 B 服务自身的发布 revision。

B 服务可以浏览 A 管理的 key 字段，但不得修改。B 服务 overlay 只对自己负责的字段生效。

### 9.3 客户端职责

客户端负责：

- 保存 B 服务 URL，支持默认占位 URL 和用户配置 URL。
- 以本地当前 revision 调用云服务 GET API。
- 校验发布 JSON schema、revision、可选签名和缓存头。
- 用三方合并算法把远端更新应用到本地配置。
- 不覆盖用户手工写入字段和本机运行统计字段。
- 原子替换 `base` 与 `local` 配置。

## 10. 字段所有权

| 字段/对象 | A 服务 | B 服务 | 客户端 |
|---|---|---|---|
| `provider_key` | 写 | 读 | 读 |
| `provider_driver` | 写 | 读 | 读 |
| `provider.name` | 写 | 读 | 匹配与展示 |
| `provider.base_url` | 写 | 读 | 以 endpoint 匹配 |
| `model.id` | 写 | 读 | 读 |
| `model.source_model_id` | 写 | 读 | 读；仅用于编辑态、diff、trace 和审计，不作为 driver metadata 下发字段 |
| `original_provider` | 写 | 读 | 读 |
| `nick` | 写 | 读 | 读 |
| `api_types` | 写 | 读，可禁用 | 读 |
| `logical_mounts` | 写 | 可 overlay 运营目录 | 本地可 overlay |
| `capabilities` | 写 | 读 | 读 |
| `context_limits` | 写 | 读 | 读 |
| `pricing.reference` | 写 | 可覆盖运营价 | 可结合动态价和汇率 |
| `routing_weight` | 读 | 写 | 本地可 overlay |
| `disabled` | 写技术禁用 | 写运营禁用 | 本地可禁用 |
| `latency/reliability` | 不写 | 不写 | 本机统计写 |
| 用户自定义 provider/model 字段 | 不写 | 不写 | 写 |

污染处理：

- A 发布 JSON 中出现 B-only 字段，B 合并时丢弃这些字段并记录 warning。
- B overlay 中出现 A-only 字段，B 发布时丢弃这些字段并记录 warning。
- 客户端远端更新中出现 local-only 统计字段，客户端丢弃这些字段。

## 11. 发布 JSON

### 11.1 顶层格式

客户端 API 返回格式：

```json
{
  "revision": "20260708.000001",
  "generated_at": 1783500000000,
  "source": {
    "service_role": "B",
    "tech_revision": "20260708.000001",
    "ops_revision": "20260708.000004"
  },
  "documents": []
}
```

`documents[]` 中的每一项都是 `11.2` 定义的 driver metadata document。API envelope 只表达传输状态、分页和 revision，不是客户端 driver metadata 的存储或消费结构。

`revision` 必须单调递增，推荐格式：

```text
<utc_yyyymmddHHMMSS>.<sequence>
```

同一毫秒内多次发布时递增 sequence。服务端每次成功发布必须同步更新 revision 和 ETag。

### 11.2 Client Driver Metadata 格式

本节描述客户端最终接收的 driver metadata document。`name` 和 `base_url` 是 provider meta 下发字段；`provider_key`、`provider_kind`、`protocol_family`、源对象引用、Nick Rules、Logical Directory、Dictionaries 和运营 overlay 字段可以参与物化计算，但不作为 driver metadata 字段下发。

```json
{
  "schema_version": 1,
  "provider_driver": "openai",
  "name": "OpenRouter",
  "base_url": "https://openrouter.ai/api/v1",
  "revision": "20260708.000004",
  "models": [],
  "patterns": [],
  "defaults": {},
  "variants": [],
  "version_rules": [],
  "signature": null
}
```

发布 JSON 是 RDB 持久数据的物化视图，不是存储结构的直接镜像。`model_param_rules` 在 RDB 中统一保存，但发布时必须按 `match_type` 拆回客户端既有字段：

- `exact` 规则输出到 `models[]`，表示最高优先级的精确模型参数。
- `pattern` 规则输出到 `patterns[]`，必须按 `priority` 从高到低稳定排序。
- `default` 规则输出到 `defaults`，表示 `models[]` 和 `patterns[]` 都未命中时使用的兜底参数。

客户端和发布预览的匹配顺序固定为：先匹配 `models[]` 中的精确 model id，再按数组顺序匹配 `patterns[]`，最后使用 `defaults`。任何 mock API、后端合成器或发布校验都不得把这三个发布字段合并成一个下发字段。

客户端创建 provider instance 时：

- 如果用户配置了 endpoint，优先用 endpoint 匹配 `provider.base_url`。
- 如果没有 endpoint，认为 endpoint 是 `provider_driver` 的原厂默认 endpoint。
- 如果预置的是非原厂 endpoint，如 OpenRouter，客户端应以 `provider.name` 展示 provider Type，并以 `provider.name` 或 `base_url` 匹配参数。
- `provider_driver` 只表示接口协议或 driver，不表示最终 UI 上的主流 provider Type。

### 11.3 Model 格式

```json
{
  "id": "openai/gpt-5.2",
  "provider": "openrouter",
  "model_driver": "openai",
  "api_types": ["llm.chat"],
  "logical_mounts": ["llm.gpt-standard", "llm.openai.gpt-5-2"],
  "capabilities": {
    "streaming": true,
    "tool_call": true,
    "json_schema": true,
    "vision": true,
    "max_context_tokens": 128000,
    "max_output_tokens": 16384
  },
  "pricing": {
    "price": 1.25,
    "currency": "USD",
    "unit": "1M_tokens"
  },
  "quality_score": 0.9,
  "cost_class": "medium",
  "latency_class": "normal",
  "exclude": false
}
```

规则：

- `id` 是客户端看到的 provider model id；存在 nick 时使用 nick。
- 原始模型 id 保存在编辑态、diff、trace 和审计记录中，不写入客户端 driver metadata rule。
- `original_provider` 必填。
- `provider` 为空时表示来自全局/原厂 `model_param_rule`；在最终 provider 发布列表里应填当前 provider key。
- exact/pattern `model_param_rule.exclude=true` 的模型仍出现在发布规则中，但以 `exclude=true` 表达排除。
- 价格是参考价或运营价；如果 provider 运行时可动态返回价格，以 provider 返回价格为准。
- 汇率不在云服务处理，由客户端统一处理。

## 12. 合成规则

### 12.1 A 服务合成 provider

对每个 provider：

1. 读取 provider 技术主数据。
2. 读取该 provider 白名单引用的 models、patterns、defaults、variants、version rules；没有引用的对象不参与该 provider 的发布。
3. 合并源对象与 provider 专属 `model_param_rules`；provider 专属规则按相同 `match_type` 和 selector 覆盖源对象。`exclude=true` 的 exact/pattern 在此阶段参与合并。
5. 将 `model_param_rules` 物化为发布 JSON 三字段：`exact` -> `models[]`，`pattern` -> `patterns[]`，`default` -> `defaults`。
6. `patterns[]` 必须按 `priority` 稳定排序，排前面的优先级高，排后面的优先级低；`defaults` 优先级最低且每个 scope 最多一条。
7. 发布预览和校验对每个候选模型执行同一匹配顺序：先按原始 `model.id` 匹配 `models[]`，再按数组顺序匹配 `patterns[]`，最后使用 `defaults`。一个模型只允许得到一条最终参数规则。
8. 应用 model nick，重写 `models[].id`；原始模型 id 进入 diff/trace/审计记录，不进入客户端 driver metadata。同时重写 `patterns[]` 中的 model selector；default 规则没有下发 selector。
9. 对 variants/version_rules 执行 provider 专属覆盖和 nick 重写：先按原始 `model.id` 或 wildcard selector 以及 `original_provider` 命中规则，再把 variant `model_id_selector` 和 version rule `content.model_pattern` 改写为发布 selector。variant 无 selector 时使用默认 selector `*`，该 `*` 也参与 nick rewrite。
10. 对没有任何 `models[]`、`patterns[]` 或 `defaults` 命中的模型阻止发布或记录 Error；正常情况下 default 规则应提供兜底。
11. exact/pattern `model_param_rule.exclude=true` 的模型保留在发布规则中并设置 `exclude=true`。
12. 对源对象的参数实际发生变更时才物化 provider 专属对象；未变更时保持 source key 引用。

Provider 不存在“默认引用全部原厂对象”的隐式规则；白名单是唯一的对象来源。default 规则也可被引用，但同一目标 provider 最终只能有一个启用 default；多个来源 default 必须先拆成可区分的 pattern 后再新建目标 default。
default、variants、version rules 只有被目标 provider 的白名单显式引用时才参与其发布。配置 provider 专属对象时，可以引用原厂记录并 patch，也可以创建完全专属记录。合成器必须允许每个 provider 下发多条 variants、version_rules。

### 12.2 B 服务合并 A 发布

B 服务合并流程：

1. 读取本地 A 服务 URL。
2. 拉取 A 发布 JSON；如果 A revision 未变化且本地 A 缓存可用，可直接使用缓存。
3. 校验 A 发布 JSON schema 和 revision。
4. 丢弃 A JSON 中不属于 A 的污染字段。
5. 对 provider/model/model_param_rule/variants/version_rules 应用本地 `ops_overlays`。
6. 对 B 禁用的 provider，最终发布不包含该 provider。
7. 对 B 禁用的 model，最终发布不包含该 model；如需要保留诊断，可在管理 API 中可见，不下发客户端。
8. 对 B 禁用的 model_param_rule/variants/version_rules，发布时移除对应项。
9. 生成 B 发布 JSON、revision、ETag 和缓存。

如果单个 provider 或 model 合并失败，只跳过该对象并记录 warning；不得因为一条污染参数丢弃整个发布文档。

## 13. 云服务 API

### 13.1 公开 GET API

```http
GET /v1/provider-driver-metadata?revision=<client_revision>&cursor=<cursor>&limit=<limit>
Accept-Encoding: br, gzip
If-None-Match: "<etag>"
```

响应：无需更新

```json
{
  "status": "not_modified",
  "revision": "20260708.000001"
}
```

响应：完整或分批更新

```json
{
  "status": "ok",
  "revision": "20260708.000002",
  "batch": {
    "cursor": null,
    "next_cursor": "driver:openai:1",
    "done": false
  },
  "documents": []
}
```

最后一批：

```json
{
  "status": "ok",
  "revision": "20260708.000002",
  "batch": {
    "cursor": "driver:openai:1",
    "next_cursor": null,
    "done": true
  },
  "documents": []
}
```

要求：

- 配置量小的时候应一次返回。
- 支持 `gzip` 和 `br`。
- 支持 `ETag`、`Cache-Control`、`Last-Modified`。
- GET 权限全开放，但要有常规防攻击措施：限速、请求大小限制、分页上限、IP 风险控制。

### 13.2 管理 API

管理 API 需要账号登录和 `metadata.update` 授权。

```http
POST /v1/admin/edit-sessions
POST /v1/admin/edit-sessions/{session_id}/import-plan
POST /v1/admin/edit-sessions/{session_id}/preview
POST /v1/admin/edit-sessions/{session_id}/approve
POST /v1/admin/edit-sessions/{session_id}/publish
POST /v1/admin/edit-sessions/{session_id}/discard
GET  /v1/admin/change-logs
GET  /v1/admin/providers
GET  /v1/admin/models
```

B 服务额外提供：

```http
GET  /v1/admin/tech-source
PUT  /v1/admin/tech-source
POST /v1/admin/tech-source/refresh
```

`tech-source` 保存技术参数服务查询 URL 和最近成功同步的发布源 revision。UI 展示名称应为“技术源”或“同步源”。

## 14. WebUI 设计

### 14.1 通用模式

技术参数和运营参数后台 WebUI 默认是浏览模式。增删改是高风险操作，必须显式切换到编辑模式。

WebUI 文案不得直接暴露 A/B 服务简称。技术事实维护界面展示为“技术参数”，运营 overlay 维护界面展示为“运营参数”；运营侧拉取技术发布结果的配置入口展示为“技术源”或“同步源”。

进入编辑模式后，普通属性字段可以按字段所有权直接编辑；key 性质字段仍以预览/只读形式展示。`provider_key`、`nick_key`、`variant_key`、`version_rule_key` 由服务端自动编号，不提供字段级修改。修改其它 key 性质属性时，发布确认页必须把 key 性质字段变更放入独立风险区，并要求管理员确认影响范围。

PC 端编辑模式主要服务高密度维护场景，Provider、Model、规则、Nick、Pattern、Variant、Version Rule 等数量较多的对象应优先使用分页表格，并提供搜索、筛选、排序和批量选择。浏览模式可以使用卡片或卡片+列表布局，以兼顾移动端阅读；但 PC 编辑主路径不能只依赖卡片或长 JSON。

进入编辑模式：

1. 保存当前发布状态快照。
2. 创建 `edit_session`。
3. UI 顶部显示 base revision、操作者和编辑状态。

草稿恢复：

1. 管理员可随时保存当前 `edit_session` 为草稿，不生成发布 revision。
2. 管理员下次登录时，WebUI 必须提示其工作区存在未完成草稿。
3. 草稿列表按 service role、base revision、updated_at、变更摘要展示。
4. 管理员可以逐个选择继续编辑、进入发布预览或放弃草稿。
5. base revision 已落后的草稿必须重新预览，重新计算 diff、影响范围和测试建议后才允许发布。

退出编辑模式：

1. 对编辑后状态和快照做 diff。
2. 展示影响范围：provider 数、model 数、model_param_rules 数、variants/version_rules 数、default 规则覆盖、逻辑目录影响、API type/capability 影响。
3. 导出 diff 文本，供人或 AI 分析核实。
4. 生成测试用例建议。
5. 二次确认后写 change log 并发布。

移动端 WebUI 只支持浏览和紧急禁用，不支持普通编辑、批量操作、导入计划和发布。

### 14.2 技术参数 WebUI

必须支持：

- 添加原厂 provider。
- 添加全局/原厂 model_param_rule。
- 以已有 provider/model 为模板创建新对象。
- 编辑 `model_param_rules`，包括 exact、pattern、default 三类模型参数匹配规则；编辑 variants/version_rules 数组元素；variants/version_rules 支持从全局/原厂记录引用后转为 provider 专属配置。
- 添加聚合模型中间商，如 OpenRouter。
- 从原厂列表、原厂模型列表选择模型构造聚合 provider。
- 为 provider 选择模型时，WebUI 必须按统一的 `model_param_rules` 处理 exact、pattern、default 三种匹配模式。管理员可以选择精确模型、选择 pattern，也可以选择 default 作为模板创建更具体的匹配规则。
- 支持跨类型复制创建新规则：从 pattern 创建 exact 时继承参数字段并把 selector 改成一个或多个精确原始 `model.id`；从 default 创建 exact/pattern 时继承 fallback 参数并补充目标 selector，pattern 还必须补充 priority。该操作创建新 `model_param_rules` 记录，不修改原记录的 `match_type`。
- 跨类型复制创建必须弹出确认提示，说明来源类型和目标类型不同，最终下发和存储以用户选择的目标 `match_type` 为准，并列出 selector、priority 等会被改写或清空的字段。
- 批量 nick：加前缀、加后缀、替换片段、pattern rewrite。
- Nick rewrite 必须支持多条规则，按 provider、original provider、exact/pattern selector 和 priority 预览最终 published selector。聚合 provider 可用不同规则分别处理不同原厂前缀；version rule 预览和发布都以 `content.model_pattern` 为通配字段。
- 批量选择：按厂商、协议族、api_type、capability、model.id 模糊匹配。
- `model_param_rules` 管理面板在 Models 页面：按全局和 provider 专属 scope 浏览、创建、复制、编辑、禁用、删除 exact、pattern、default 三类规则。exact 精确匹配 `model.id`；pattern 按 selector 和 priority 顺序匹配；default 每个 scope 最多一条启用记录。三类规则必须统一保存在 `model_param_rules` 中，不能拆表；PC 编辑模式优先使用分页表格，详情使用结构化表单；JSON 视图仅作为辅助检查器。exact/pattern/default 使用独立列配置、筛选项、详情面板、创建表单、编辑表单和 Zod/schema 校验。
- `exclude` 是 exact/pattern model_param_rule 的属性。`exclude=true` 时该规则的其它参数字段不参与当前发布语义，但保留在记录中，取消 exclude 后可恢复；default 不设置 exclude。
- Provider 全量 JSON 视图和导出必须按发布格式展示 `models`、`patterns`、`defaults` 三个字段，不得只展示统一存储后的 `model_param_rules`。
- Resolver Rules 页面只管理 variants/version_rules：按全局和 provider 专属 scope 浏览、创建、复制、编辑、禁用、删除，支持每个 provider 多条记录。
- 逻辑目录管理：目录树和面包屑浏览；支持调整目录树结构，新增、删除、重命名和修改子目录属性；支持批量添加/移除模型到目录，并支持一个模型挂到多个目录。内容区最上方必须提供筛选检索，可筛选目录和目录下包含的模型；筛选检索模式与按目录路径浏览模式互斥；右侧详情区展示选中目录或模型的详情。删除/移动目录前必须展示受影响模型数和路径样例，目录 key/path 重复、空目录和断链引用必须 warning 或阻止提交。
- api_type 管理面板：维护 api_type 字典并支持新增；删除和重命名属于低频高风险操作，需要显示引用模型数量和影响样例；支持按筛选结果给一批模型标识支持某个 api_type，也支持给单个模型添加多个 api_type。应用 api_type 时必须通过下拉框、combobox、单选/多选或等价控件选择已有字典项，不能通过自由文本提交不存在的 key。
- capabilities 管理面板：维护 capability 字典、类型和默认展示信息；删除和重命名属于低频高风险操作，需要显示引用模型数量和影响样例；支持按筛选结果给一批模型标识支持某个 capability，也支持给单个模型添加多个 capability。应用 capability 时必须通过下拉框、combobox、单选/多选或等价控件选择已有字典项；大多数属性按 bool 语义表达为支持/不支持，少数值属性必须使用结构化输入并带单位、范围和 schema 校验。
- 删除/新增全局/原厂 model_param_rule 前显示受影响 provider 列表。

Authoring workflow requirements:

- 创建 provider 时服务端分配 `provider_key`；模板只能是一个具体 provider 或“从零创建”。`provider_kind` 仅为 `origin` 或 `aggregator`。编辑现有 provider 复用创建向导，回填该 provider 的完整 scoped set，且不允许修改 `provider_key`。
- `provider_driver` 是 driver metadata document 内的 `provider_driver` 值，不是文件名；`protocol_family` 是客户端调用 endpoint 的 wire protocol。表单选项从加载的 metadata 派生，因此 fal、minimax 等 driver 不会因固定选项遗漏。
- Models 是 source-rule 白名单选择，不创建规则。左侧按来源 provider 展示已选/总数，中部按 Models/Patterns/Defaults Tab 选择现有对象，右侧按相同分类展示待引用对象。default 也是可选来源对象。
- Model params 的上半区使用同一三栏目标选择器，下半区编辑目标参数。多选时相同字段显示共同值，不同字段为空；只有明确赋值的字段被批量覆盖，未赋值字段保留每个对象的原值。`original_provider` 全部为当前 provider 时只读显示，否则只能通过按钮设为当前 provider；`model_id_selector` 必须逐条赋值；Model params 不编辑 priority。Apply 提交当前批次并清空目标，Discard 恢复当前批次开始时的值。只有内容实际不同于来源时才物化 provider 专属对象。`max_context_tokens` 仅在对应 capability 已选中时显示输入。
- Models、Patterns、Defaults 的匹配身份在每个 provider 内唯一，且最多一个启用 default。多个来源 default 必须先各自转为可区分 pattern，再创建目标 default。Pattern order 是独立步骤，列表顺序是唯一的 authoring 优先级表达，发布时转换为稳定 `priority`。
- Variants / Version rules 与 Models 使用相同的三栏白名单选择，但只选择已有对象。Variant/version params 分为两个类型 Tab，各自使用字段面板；每个 Tab 的待编辑对象选择方式与 Model params 一致，支持多选、Pending edit list、Discard 和 Apply。类型不可原地修改，本步骤不编辑 priority，也不批量编辑 logical mounts/auto_mounts。Variant 面板编辑 `provider_options`；Version rule 面板编辑完整谓词和参数，`tier_tokens`、`exclude_tier_tokens`、`stability.unstable_tokens` 使用自由文本 token 输入，`current_mount` 和 `version_mount` 通过物化目录树单选。Version rule 的列表、Inspector 和预览必须展示完整匹配谓词，不只显示 `model_pattern`。
- Logical mounts 是 Add Provider 向导中唯一批量配置 `logical_mounts`/`auto_mounts` 的步骤。目标选择器覆盖非空的 Models、Patterns、Defaults、Version rules Tab，且允许跨 provider/Tab 选中；Variants 在该步骤暂不配置。目录树对全部目标为已选、部分目标为不确定、未选三态；右侧 Selected paths 只展示全部目标实选路径。Apply 时实选路径添加到全部目标，未选路径从全部目标移除，部分选中路径保持各目标原状；Discard 丢弃尚未 Apply 的路径选择变更并恢复目录树状态。Version rule 的 `current_mount`/`version_mount` 只在参数面板中作为单值挂载字段选择。
- Nick rewrite 是 source-to-published 的发布期映射，支持 origin-prefix 与 exact/pattern mapping，可有多条规则，作用于 models、patterns、variants、version rules。exact models 重写 `models[].id`，patterns 重写 `patterns[].pattern`，default 无下发 selector；没有 selector 的 variant 以 `*` 参与匹配，version rules 重写 `content.model_pattern`。Nick Rules 是同一编辑器的跨 provider 页面，按 provider 检索；创建时指定 provider，编辑时 provider 不可变，`nick_key` 自动编号。
- 来源对象的任何属性变更或删除都必须根据 `source_rule_key`、`source_variant_key`、`source_version_rule_key` 找到依赖 provider，并产生同步审查告警。
- Providers、Models、Nick Rules、Resolver Rules、Dictionaries 的浏览模式 Inspector 仅展示属性。编辑模式在行内提供编辑和删除。Resolver Rules 分 Variants、Version rules 两个 Tab，支持按 provider 检索；创建时指定 provider/original provider，后续不可变更；variant/version rule key 自动编号。
- Logical Directory 的面包屑每一级可点击，根路径拼接不得产生 `//`；目录浏览列表必须展示当前目录的全部直属子目录和模型。Inspector 明确展示选中对象的身份、来源、挂载路径与引用关系。该页面不提供按目录批量添加模型。

### 14.3 运营参数 WebUI

必须支持：

- 配置技术参数服务 URL。
- 查看发布源 revision 与运营 revision。
- 浏览技术参数字段，但这些字段只读。
- 为 provider/model/model_param_rule/variants/version_rules 增加运营 overlay。
- 禁用 provider/model。
- 批量调整首版固定运营字段：disabled、pricing override、routing weight、recommendation level、display priority。
- 查看污染字段 warning。
- 预览最终下发给客户端的发布 JSON。

## 15. 导入文本格式

导入文本使用人类可读 YAML。文件可以由人编辑，也可以由 AI 生成。WebUI 导入后，应把各条 action 分发到对应 UI 组件，进入待提交状态，而不是直接发布。

```yaml
schema_version: 1
kind: provider_metadata_update_plan
target_service: A
title: add openrouter gpt models
author: human-or-ai
base_revision: "20260708.000001"

actions:
  - action: upsert_provider
    provider_key: openrouter
    provider_driver: openai-compatible
    name: OpenRouter
    base_url: https://openrouter.ai/api/v1
    provider_kind: aggregator

  - action: include_models
    provider_key: openrouter
    selector:
      original_provider: openai
      model_id_pattern: "gpt-*"

  - action: set_model_nick
    provider_key: openrouter
    selector:
      original_provider: openai
      model_id_pattern: "gpt-*"
    rewrite:
      prefix: "openai/"

  - action: upsert_model_param_rule
    provider_key: openrouter
    match_type: exact
    original_provider: openai
    model_id_selector: gpt-5.2
    params:
      pricing:
        price: 1.4
        currency: USD
        unit: 1M_tokens

  - action: set_logical_mounts
    selector:
      original_provider: openai
      model_id_pattern: "gpt-5*"
    mode: add
    logical_mounts:
      - llm.gpt-standard

  - action: disable_model
    provider_key: openrouter
    selector:
      id: openai/gpt-4.1-preview
    reason: preview model is not recommended
```

支持的 action：

- `upsert_provider`
- `disable_provider`
- `upsert_model_param_rule`
- `delete_model_param_rule`
- `include_models`
- `exclude_models`
- `set_model_nick`
- `upsert_variant`
- `upsert_version_rule`
- `set_logical_mounts`
- `upsert_logical_directory`
- `delete_logical_directory`
- `move_logical_directory`
- `set_api_types`
- `upsert_api_type`
- `delete_api_type`
- `set_capabilities`
- `upsert_capability`
- `delete_capability`
- `set_pricing`
- `set_ops_overlay`
- `disable_model`

导入校验：

- `target_service=A` 的文档不得包含 B-only action。
- `target_service=B` 的文档不得修改 A-only key 字段。
- 所有 selector 必须可预览命中结果。
- `upsert_model_param_rule` 必须支持 `match_type`、`source_rule_key`、`original_provider`、`model_id_selector`、`priority` 和 params 字段；`exact` 必须设置原始 `model.id`，`pattern` 必须设置 selector 和 priority，`default` 不得设置 selector 和 priority。导入预览必须展示 exact -> pattern(priority) -> default 的命中链路和样例。
- `upsert_variant` 必须支持 `source_variant_key`、`selector_type`、`original_provider`、`model_id_selector`、`priority` 和 `nick` 字段；其中 `model_id_selector` 使用原始 `model.id`，导入预览必须展示 nick 重写后的发布 selector。
- `upsert_version_rule` 必须支持 `source_version_rule_key`、`selector_type`、`original_provider`、`model_pattern`、`priority` 和 `nick` 字段；其中 `model_pattern` 使用原始 `model.id` 或 wildcard pattern，导入预览必须展示 nick 重写后的发布 selector。兼容展示用的 `model_id_selector` 只作为 `content.model_pattern` 的镜像，不作为 version rule 的主通配字段。
- 任何批量 action 在提交前必须显示命中数量和样例。
- 修改 key 性质字段、删除/重命名 api_type 或 capability、删除/移动逻辑目录时，提交前必须显示引用关系、受影响对象数量和风险提示。

## 16. 客户端更新流程

### 16.1 本地文件分层

客户端维护两份基础状态：

- `base`：上次成功应用云更新后的基础配置。
- `local`：当前系统生效配置，包含用户修改和本机统计。

下载得到：

- `remote`：云端新 revision 对应的发布配置。

### 16.2 三方合并

流程：

1. 对 `base` 和 `local` 取快照。
2. 下载 `remote`。
3. 校验 `remote`。
4. 计算 `diff_local = diff(local, base)`。
5. 计算 `diff_remote = diff(remote, base)`。
6. 从 `local` 拷贝出 `merged`。
7. 逐条应用 `diff_remote`：
   - 如果同一路径出现在 `diff_local` 中，说明用户改过，跳过该字段。
   - key 性字段永远不改。
   - local-only 统计字段永远不改。
   - remote 新增字段在 local 未改过时补充。
   - remote 删除对象时，如果用户只改了统计字段，不阻止删除。
8. 原子写入新的 `base = remote` 和 `local = merged`。
9. 更新本地 revision。

注意：概要中的“用 remote 替换 local”应理解为“以 remote diff 为输入生成 merged local 后，原子替换本地生效配置”，不能直接覆盖用户配置。

### 16.3 数组 diff 规则

数组必须有元素身份 key：

| 数组 | identity |
|---|---|
| driver metadata document | `provider_driver`。 |
| `models[]` | `id`。 |
| `patterns[]` | `pattern_key` 或 `pattern`。 |
| `variants[]` | `name`。 |
| `version_rules[]` | `rule_key` 或 `family + tier + model_pattern`。 |
| `logical_mounts[]` | 字符串值。 |
| `api_types[]` | 字符串值。 |
| `capabilities` | object path。 |

数组顺序变化本身是 diff。数组元素顺序变化不能阻止 remote 修改该元素属性；数组元素属性变化也不能阻止 remote 调整数组顺序。但如果用户修改了某个元素的非统计字段，remote 删除该元素时应保留该元素，除非 remote 明确是安全删除且用户只改了 local-only 统计字段。

### 16.4 增删边界

- 用户手工添加 provider/model：remote 不得删除；remote 可补充缺失字段或数组项。
- 用户物理删除 provider/model：remote 不得再添加回来。
- 用户只是设置删除标记：remote 可更新其属性，但保留删除标记。
- 用户手工添加或删除某字段/数组项：remote 不更新对应字段/数组项，但可更新同数组其他项。
- 本机统计字段变化不阻止 remote 删除对应 provider/model。

### 16.5 Provider 信息页功能

客户端 provider 信息页提供：

- `恢复配置`：把该 provider instance 中用户编辑过的字段还原成 base。执行前展示将改变的字段和风险提示。
- `导出配置`：把 provider instance 配置导出为可读文本，可作为新建自定义 provider 的模板。

## 17. 自定义 Provider 流程

用户在客户端添加自定义 provider：

1. 输入 provider Type 和 endpoint。
2. 如果 endpoint 匹配 `provider.base_url`，启用对应云配置。
3. 如果没有 endpoint，则按 provider Type 的原厂默认 endpoint 匹配。
4. 如果匹配不到：
   - 提示风险。
   - 提供高级配置向导。
   - 可导入文本计划。
   - 可调用 provider `/models` 获取模型列表，并把返回列表作为候选源模型。
   - 提供 nick 编辑器，把返回模型 id 匹配到原厂模型 meta。

OpenRouter 示例：

- `provider.name = OpenRouter`
- `provider.base_url = https://openrouter.ai/api/v1`
- `provider.provider_driver = openai-compatible` 或当前实现确认后的 driver id
- UI provider Type 展示 `OpenRouter`
- 实际调用协议使用 OpenAI-compatible driver

## 18. 缓存、压缩与可用性

服务端：

- 发布后生成不可变发布缓存。
- GET API 优先返回缓存。
- 支持 `ETag`、`If-None-Match`、`Cache-Control`。
- 支持 gzip/br。
- 运营参数服务拉取技术源失败时，可继续使用最近一次成功的技术源缓存，并在管理端提示 stale。
- Stale 状态下允许运营参数发布；发布确认页必须展示 stale 风险，`change_logs` 必须记录本次发布使用的发布源 revision 和 stale 状态。
- 如果当前 RDB 数据损坏，公开 GET API 可继续提供上一版发布缓存；管理写入暂停。

客户端：

- 网络失败时继续使用本地配置。
- remote schema 校验失败时丢弃 remote，保留现有 base/local。
- 分批下载未完成时不得应用。
- 只有所有 batch 校验完成后才进入三方合并。

## 19. 安全与权限

公开 GET：

- 无认证。
- 限速、防爬、请求参数长度限制、分页上限。
- 只返回公开 metadata，不返回账号、密钥、内部备注、编辑日志。

管理端：

- 需要账号系统认证。
- 更新配置需要 `metadata.update` 权限。
- 发布需要二次确认。
- 修改 key 性质字段需要字段级解锁和发布确认页风险确认，不需要独立审批流。
- 移动端管理界面只允许浏览和紧急禁用，不允许普通编辑和发布。
- 所有发布写入 `change_logs`。
- 可选增加签名：发布 JSON 中保留 `signature` envelope；客户端可按策略启用校验。

## 20. 对接 AICC

云端发布结果落到客户端后，应进入当前 AICC driver metadata 体系：

```text
remote publish JSON
-> $BUCKYOS_ROOT/etc/aicc/driver_metadata/remote_cache/<driver>.json 或等价缓存
-> metadata_resolver
-> ProviderInventory
-> ModelRegistry::apply_inventory()
-> logical_mounts/default items/auto admission
-> ModelRouter/ModelScheduler
```

需要保持的现有约束：

- provider 自发现只负责模型 id。
- driver metadata resolver 负责能力、挂载、variant、成本 fallback。
- unknown model 走 conservative fallback。
- `ModelCapabilities` 是能力真相源。
- `logical_mounts` 受 `LogicalModelDefinition.min_line` admission 约束。

## 21. 验证计划

单元测试：

- A 服务 provider/model/rule 合成。
- provider 专属 model_param_rule 覆盖全局/原厂规则。
- 专属 meta 等价时自动删除。
- nick 后 `models[].id` 重写；原始 id 仅在 diff/trace/审计中保留。
- `patterns[].pattern`、variant `model_id_selector`、version rule `content.model_pattern` 能按 provider nick 重写，并在冲突或无法重写时阻止发布。
- exact/pattern `model_param_rule.exclude=true` 的模型发布为 `exclude=true`。
- patterns 顺序匹配。
- B 服务污染字段丢弃。
- B 服务禁用 provider/model 后不下发。
- revision 递增与 ETag 更新。
- key 性质字段变更会进入独立 diff 风险区，并要求二次确认。
- model_param_rules 面板编辑后能通过 schema 校验，并能按 exact -> pattern(priority) -> default 生成最终参数分类；variants/version_rules 面板编辑后能通过 schema 校验、命中预览、nick 重写预览和发布合成；每个 provider 每种 variants/version_rules 可发布多条记录。
- 批量 api_type/capability 标识能正确更新命中模型，单模型多个 api_type/capability 能正确发布。

客户端测试：

- `not_modified` 不更新。
- 单 batch 更新。
- 多 batch 完整后再应用。
- 三方合并不覆盖用户字段。
- 本机统计字段不被 remote 覆盖。
- 用户物理删除对象不被 remote 添加回来。
- 数组顺序 diff 和元素属性 diff 互不误伤。
- remote 删除对象时，本机统计字段变化不阻止删除。

集成测试：

- B 从 A 拉取并合并发布。
- 客户端拉取 B 发布后 AICC `models.list` 可看到新 provider/model。
- `logical_mounts`、`api_types`、`capabilities`、`pricing` 正确进入 `ProviderInventory`。
- 逻辑目录树新增、删除、重命名、移动子目录后，发布结果与客户端逻辑目录浏览一致。
- OpenRouter 以 `provider.name` 展示 provider Type，调用 driver 使用 OpenAI-compatible。
- 损坏 A 缓存不影响 B 上一版发布 GET。

## 22. 落地顺序

1. 固化发布 JSON schema 和本地 remote cache 路径。
2. 先实现独立 WebUI mock-first 原型：在独立前端包内参考 desktop 的 Shell、导航、页面模块、向导模块、共享组件和 i18n 组织方式，使用 `src/frame/aicc/driver_metadata/*.json` 构造 mock 数据，不接真实后端，不注册到 desktop，不污染其他模块。mock seed 必须按本文定义的 RDB 表格/数据集合组织，页面需要的计数、命中数量、发布样例、风险摘要、目录树、搜索结果和最终 JSON 片段由 mock API/selectors 计算得到；这些 mock API 边界后续演进为真实后端 API。具体模块划分见 `product/ai_center/Provider_Metadata_Cloud_WebUI_PRD.md` 的“前端模块划分与代码组织”。
3. 实现 A 服务 RDB schema、合成器和公开 GET API。
4. 实现 B 服务 A URL 配置、A 拉取缓存、overlay 合并器和公开 GET API。
5. 将 WebUI mock API 替换为 A/B 服务真实接口，保留已拆分的页面、向导和共享组件边界。
6. 实现编辑模式、快照、diff、二次确认和 change log 的后端持久化。
7. 实现导入文本计划。
8. 客户端实现 revision 拉取、分批下载和三方合并。
9. 对接 AICC metadata resolver reload。
10. 补齐验收测试和回滚策略。

## 23. 风险与待确认

待确认：

- `provider_driver = openai-compatible` 是否作为正式 driver id，还是继续使用 `openai` 表示 OpenAI-compatible 协议。
- 发布 JSON 是否按 provider_driver 拆分落入现有 `$BUCKYOS_ROOT/etc/aicc/driver_metadata/remote_cache/<driver>.json`，还是新增聚合缓存文件后由 resolver 拆分。
- exact、pattern、default 已明确统一存储为 `model_param_rules`，其中 default 是 exact 和 pattern 全部失败后的兜底匹配规则；variants、version_rules 分别存储为 `metadata_variants`、`metadata_version_rules`。落地时需要同步更新 schema、resolver 和发布 JSON 物化逻辑。

主要风险：

- provider/model key 设计不稳定会导致客户端三方合并误判。
- nick 会改变客户端看到的 model id，必须在 diff 和 trace 中始终保留原始 model id，但该 trace 字段不进入客户端 driver metadata JSON。
- model_param_rules 的 pattern selector、variant selector、version rule `content.model_pattern` 若未同步 nick 重写，会出现模型存在但规则失效的隐蔽问题，必须在发布预览中列出重写前后样例。default 规则不参与数组式 selector 重写，但必须展示最终发布值。
- 聚合商 provider 的候选源模型集合可能来自 `/models` 动态返回，必须和原厂 meta 匹配失败场景一起设计 UI。
- exact、pattern、default 如果拆成多表，会增加匹配优先级和最终命中一致性风险；必须统一存储在 `model_param_rules` 中。variants、version_rules 仍需按独立表和独立 schema 做强校验。
- api_type/capability 如果允许自由文本应用，会产生拼写错误和能力标记漂移；WebUI 和服务端都必须做字典引用校验。
- 客户端合并算法复杂，尤其是数组顺序和元素删除，需要独立测试覆盖。
- A/B 字段所有权如果没有机器校验，长期会产生污染字段。

## 24. Origin Identity 和 schema v2 更新（beta 2.2）

客户端最终消费的 provider-driver metadata 从 beta 2.2 开始使用 `schema_version: 2`。下发 JSON 保留当前服务渠道字段，例如 `provider_driver`、`protocol_family`、`base_url`、`models`、`patterns`、`defaults`、`variants`、`version_rules`；同时新增 `origin_provider_aliases` 和 `origin_mappings`，用于把 provider-native model id 解析成模型原厂身份。

`{driver}` 和 `{model}` 在 logical mounts 中表示 origin identity：`{driver}` 是模型原厂 provider，`{model}` 是模型原厂命名。当前服务渠道如果需要在内部引用，应使用 provider 记录自身的 `provider_driver` 和规则中的 provider-native selector，不再把聚合商 model id 直接当作逻辑路径身份。

`origin_mappings` 是发布阶段物化结果。Cloud WebUI 使用独立的 `origin_mapping_rules` authoring 集合维护它，不再复用历史 `model_nicks`。`model_nicks` 继续表示 Nick rewrite，只负责把 source model id 重写为 provider-native selector。最终下发不包含 `model_nicks`、`nick_key`、`origin_mapping_rules` 等 WebUI 内部字段。

`origin_mappings` 规则使用标准 regex 语法，命名捕获组固定为 `(?<driver>...)` 和 `(?<model>...)`。规则模板中的占位符只使用 `<driver>` 和 `<model>`，不复用 driver metadata 其它字段里常见的 `{driver}` / `{model}`，以避免和 JSON 模板、regex 转义规则混淆。实现必须校验 regex 合法性，并在发布预览中展示规则命中样例。

`origin_provider_aliases` 是 provider scoped 的归一化表，用于把聚合平台返回的 provider 前缀映射到系统认可的 origin driver id。建议服务端持久化结构包含 `provider_key`、`alias`、`driver`、`created_at`、`updated_at`；发布 JSON 中物化为 `{ [alias]: driver }`。例如 OpenRouter 可将 `google-gemini` 归一化为 `google`，将 `x-ai` 归一化为 `xai`。

dynamic alias 暂不进入本协议。聚合商提供的软链接模型视为 provider 侧 alias，当前阶段由 BuckyOS 自身逻辑目录系统表达软链接关系；provider metadata 下发只保留物理模型身份。需要排除的 alias 或非物理模型通过 `models[].exclude=true` 或 `patterns[].exclude=true` 表达，不新增 `exclude_patterns` 字段。

需要补充的服务端/客户端验证：

1. 发布 JSON schema v2 必须包含并校验 `origin_provider_aliases` 和 `origin_mappings`。
2. OpenRouter `openai/<model>` 与 OpenAI 官方 `{model}` 应解析到相同 origin identity。
3. regex 必须是合法标准语法，且只能通过命名捕获组产出 `driver` 和 `model`。
4. logical mounts 使用 origin identity 后，当前服务渠道 model id 不应改变最终逻辑路径。
5. dynamic alias 排除规则必须通过 exact/pattern `exclude=true` 覆盖。
