# AICC Provider Architecture — Durable Data Schema

## 1. Overview

服务：AICC。

关联设计：

- GitHub Issue `buckyos/buckyos#579`
- [provider_profile_schema.md](provider_profile_schema.md)
- [driver_metadata_update_protocol.md](driver_metadata_update_protocol.md)
- [driver_metadata_update_storage.md](driver_metadata_update_storage.md)

本文定义 Model Driver catalog、Provider mapping/rules catalog、Pricing catalog、已知服务商 catalog、Provider Instance 配置和 Provider Instance 级 inventory LKGS 的持久数据边界。目标是让静态模型语义、渠道映射、价格、动态可用性和实例私有配置分别拥有唯一真相源，并能在服务重启、catalog 更新失败或 discovery 失败后确定性恢复。

## 2. Data Classification

### Durable Data（持久数据）

| 数据项 | 所有者 | 说明 |
| --- | --- | --- |
| Provider Instance 配置 | system-config | 用户配置的实例名称、Provider profile、protocol adapter、endpoint、区域及凭据引用 |
| catalog observed 水位 | AICC | 各更新源已经验证的 index/manifest revision，防止回滚 |
| catalog 内容对象 | AICC | Model Driver、Provider rules、Pricing、Known Provider catalog 的不可变内容寻址对象 |
| catalog activation | AICC | 一次完整 catalog 发布的原子生效点及 LKGS |
| Provider inventory LKGS | AICC RDB | 每个 Provider Instance 最近一次成功 discovery 并解析后的动态库存快照 |

Provider Instance 中不保存明文凭据；只保存 system-config 现有 locked value 或 credential reference。catalog activation 不能修改 Provider Instance 数据。

### Disposable Data（可丢弃数据）

| 数据项 | 存储 | 重建方式 |
| --- | --- | --- |
| catalog staging、`.part` | AICC cache | 下次更新重新下载 |
| 已编译 exact/pattern 索引 | 内存 | 从 active catalog 重建 |
| adapter operation registry | 进程内静态注册 | 服务启动时从代码重建 |
| 当前 Provider health、队列和短期错误率 | 内存 | 运行时重新采集 |
| refresh 退避计数 | 内存 | 重启后重新开始 |
| resolved call | 请求生命周期内存 | 每次调用重新解析 |

## 3. Storage Strategy

### 3.1 Provider Instance 配置

Provider Instance 配置是 Zone 级配置，继续存储在 system-config，由 control-panel 写入、AICC 只读。AICC 不复制实例配置到本地数据库，避免两个配置真相源。

### 3.2 Catalog 对象与 activation

catalog 是不可变、内容寻址、无复杂查询的 NDN JSON 对象，沿用 `$BUCKYOS_ROOT/data/srv/aicc/provider_catalog/remote_cache/v2/<source-key>/` 文件系统布局：

```text
<source-key>/objects/<FileObject-ObjId>.json
<source-key>/objects/<FileObject-ObjId>.sha256
<source-key>/activations/<manifest-revision>.json
<source-key>/observed/index/<index-revision>.json
<source-key>/observed/manifest/<manifest-revision>.json
<source-key>/staging/<attempt>/...
<source-key>/last_used
```

这是直接使用文件系统的显式例外。理由与现有 driver metadata v1 相同：对象不可变、内容寻址、通过 NDN ObjId 直接读取，不需要结构化查询；activation 单文件是唯一提交点，可避免 RDB head 与对象文件之间的双提交。所有 durable 文件必须使用写临时文件、`sync_all`、原子 create-if-absent 和父目录同步流程。

### 3.3 Provider inventory LKGS

inventory LKGS 是按实例查询和替换的结构化状态，必须使用平台提供的 RDB instance，不绑定具体数据库后端。每个 Provider Instance 只保留一份已验证的最近成功快照；历史 inventory 不作为审计日志长期保存。

## 4. Schema Definitions

### 4.1 Object Type: Provider Catalog Index

Description：声明客户端可选择的 catalog protocol track。

Naming Convention：固定路径 `/aicc/provider-catalog/index.json`。

Content Format：UTF-8 JSON。

Content Schema：

- `format: string`，固定为 `buckyos.aicc.provider-catalog-index`。
- `index_version: u32`，v2 初始值为 `2`。
- `index_revision: u32`，同 major 下的可选字段 revision。
- `index_revision_seq: u64`，更新源内严格递增。
- `required_features: string[]`。
- `tracks[]`：`protocol_version`、`protocol_revision`、`revision_seq`、`required_features`、`manifest.path`、`manifest.obj_id`。

### 4.2 Object Type: Provider Catalog Manifest

Description：一次原子发布所包含的全部 active catalog 对象。

Naming Convention：`v2/manifest-<revision_seq>.json`。

Content Format：UTF-8 JSON。

Content Schema：

- `format: string`，固定为 `buckyos.aicc.provider-catalog-manifest`。
- `protocol_version: u32`，固定为 `2`。
- `protocol_revision: u32`。
- `revision_seq: u64`。
- `required_features: string[]`。
- `files[]`：`catalog_kind`、`catalog_id`、`path`、`schema_version`、`revision_seq`、`obj_id`。
- `tombstones[]`：`catalog_kind`、`catalog_id`、`revision_seq`。

`catalog_kind + catalog_id` 在 manifest 中唯一。`catalog_kind` 只允许 `model_driver`、`provider_rules`、`pricing`、`known_provider`。

### 4.3 Object Type: Model Driver Catalog

Description：模型静态技术语义的唯一真相源。

Naming Convention：`v2/model-drivers/<model_driver_id>-<revision_seq>.json`。

Content Format：UTF-8 JSON。

Content Schema：

- `format: "buckyos.aicc.model-driver-catalog"`
- `schema_version: 1`
- `schema_revision: u32`
- `revision_seq: u64`
- `model_driver_id: string`
- `required_features: string[]`
- `models: ModelRule[]`
- `patterns: ModelRule[]`，有序、首个命中生效
- `defaults: ModelSemanticDefaults`
- `variants: ModelVariant[]`
- `version_rules: VersionRule[]`

ModelRule 只允许模型技术字段：`id/pattern`、`parameter_scale`、`api_types`、`logical_mounts`、`capabilities`、`quality_score`、`version_rules` 引用和可选保守默认价格。禁止 endpoint、认证、protocol adapter、operation、Provider 请求参数、availability、实例健康状态和对象内嵌签名。Catalog 真实性由 NDN 对象链和 manifest ObjId 绑定保证。

### 4.4 Object Type: Provider Rules Catalog

Description：连接 Provider 渠道模型 ID、Model Driver 和已注册 operation 的规则。

Naming Convention：`v2/provider-rules/<provider_profile_id>-<revision_seq>.json`。

Content Format：UTF-8 JSON。

Content Schema：

- `format: "buckyos.aicc.provider-rules-catalog"`
- `schema_version: 1`
- `schema_revision: u32`
- `revision_seq: u64`
- `provider_profile_id: string`
- `metadata_drivers: optional string[]`
- `origin_provider_aliases: object<string,string>`
- `origin_mappings: OriginMapping[]`
- `models: ProviderModelRule[]`
- `patterns: ProviderModelRule[]`，有序、首个命中生效
- `variants: ProviderVariantRule[]`

ProviderModelRule 可包含 `match_source`、`exclude`、`operations`、`provider_options`、`request_rules`、`pricing_ref`、`remove_api_types`、`remove_features`、`estimated_latency_ms`、`latency_class`、`cost_class`。配置只能收窄 Model Driver 能力。

`metadata_drivers` 缺失表示使用内置 adapter 候选范围；显式空数组表示不匹配任何 Model Driver。空对象 `{}` 是合法的配置型 Provider override，表示全部使用程序默认规则。

### 4.5 Object Type: Pricing Catalog

Description：Provider 渠道默认价格和条件价格规则。

Naming Convention：`v2/pricing/<pricing_catalog_id>-<revision_seq>.json`。

Content Format：UTF-8 JSON。

Content Schema：

- `format: "buckyos.aicc.pricing-catalog"`
- `schema_version: 1`
- `schema_revision: u32`
- `revision_seq: u64`
- `pricing_catalog_id: string`
- `offerings: PricingOffering[]`

PricingOffering 使用 `provider_profile_id`、可选 `provider_model_id/origin_model_id`、可选 pricing context 和价格。价格支持 token、request、image、audio_second、video_second，以及基于归一化请求字段的有序条件规则。它不能包含凭据、实例 endpoint 或实例名称。

### 4.6 Object Type: Known Provider Catalog

Description：管理 UI 使用的已知服务商列表。

Naming Convention：`v2/known-providers/<catalog_id>-<revision_seq>.json`。

Content Format：UTF-8 JSON。

Content Schema：

- `format: "buckyos.aicc.known-provider-catalog"`
- `schema_version: 1`
- `schema_revision: u32`
- `revision_seq: u64`
- `catalog_id: string`
- `providers[]`：`provider_profile_id`、`display_name`、`base_url`、`protocol_adapter_id`、可选 `provider_rules_id`、可选 UI hints。

该 catalog 只提供默认值。保存 Provider Instance 前必须让用户看到并允许修正协议和 endpoint，并执行连接与协议验证。

### 4.7 External Object: Provider Instance Config

Description：system-config 中由用户管理的实例私有配置。

Content Schema：

- `provider_instance_name: string`，Zone 内唯一且不可由 catalog 更新修改。
- `provider_profile_id: string`，专用 Provider 或 `custom`。
- `protocol_adapter_id: string`，必须来自运行时注册表。
- `endpoint: string`。
- `credential_ref/locked credential fields`。
- 可选 `region/account/pricing_context`。
- 可选 `provider_rules_id` 和实例级 rules/pricing override。

### 4.8 Table: aicc_provider_inventory_lkgs

Description：每个 Provider Instance 最近一次成功 discovery 后的已验证 inventory。

| Column | Type | Nullable | Default | Description |
| --- | --- | --- | --- | --- |
| provider_instance_name | TEXT PK | NO | | Provider Instance ID |
| schema_version | INTEGER | NO | 1 | 行中 snapshot JSON 的 schema major |
| provider_profile_id | TEXT | NO | | 生成快照时使用的 Provider profile |
| protocol_adapter_id | TEXT | NO | | 生成快照时使用的 adapter |
| catalog_activation_revision | INTEGER | NO | | 解析所用 catalog activation revision |
| inventory_revision | TEXT | YES | | Provider discovery revision |
| discovered_at_ms | INTEGER | NO | | 最近成功 discovery 时间 |
| snapshot_json | TEXT | NO | | 完整 `ProviderInventorySnapshot` JSON |
| snapshot_sha256 | TEXT | NO | | snapshot_json 的 SHA-256 |
| created_at_ms | INTEGER | NO | | 首次保存时间 |
| updated_at_ms | INTEGER | NO | | 最近原子替换时间 |

Indexes：

- `idx_aicc_provider_inventory_lkgs_updated` ON `aicc_provider_inventory_lkgs(updated_at_ms)`：维护和诊断。
- `idx_aicc_provider_inventory_lkgs_activation` ON `aicc_provider_inventory_lkgs(catalog_activation_revision)`：catalog 更新后查找需重解析的实例。

Constraints：

- `provider_instance_name` 非空。
- `schema_version = 1`。
- revision 和时间戳非负。
- 写入前 snapshot 必须通过 schema、能力收窄、operation registry 和 catalog reference 校验。
- 单实例使用事务原子 upsert，失败保留旧行。

`ProviderInventorySnapshot` 包含原始 `provider_model_id`、解析后的 `model_uid/model_driver_id/origin_model_id`、动态 availability/deprecated/remote methods/pricing、静态能力交集、catalog/rule revision。不得包含凭据。

## 5. Schema Version

- catalog 本地目录版本：`v2`。
- catalog protocol major：`2`。
- 四类 catalog 对象初始 `schema_version`：`1`。
- inventory LKGS table row `schema_version`：`1`。
- Provider Instance config schema 由 system-config 对应 settings 文档维护，本架构切换后使用新字段，不读取旧 `provider_driver` 兼容别名。

`schema_revision` 只增加具有明确缺省行为的可选字段；解释语义不兼容时提升 `schema_version`。catalog protocol 或本地原子提交语义变化时提升目录/protocol major。

## 6. Upgrade Compatibility Strategy

当前版本为 beta 2.2 breaking change，采用 No-compat：

| 数据项 | 策略 |
| --- | --- |
| 旧 driver metadata v1 cache | Rebuild；v2 不读取、不迁移，旧目录可在安装清理策略中删除 |
| Provider catalog v2 objects/activation | No-compat；首次实现使用全新 v2 namespace |
| inventory LKGS | Rebuild；schema 不匹配或摘要无效时删除该实例行并重新 discovery |
| Provider Instance config | No-compat；control-panel 与 AICC 同步切换到新字段 |
| staging/运行时索引 | Rebuild |

inventory 行迁移或重建失败不能阻止 AICC 使用已验证的内置 default inventory；不得把无效旧行标记为最新成功快照。

## 7. Extensibility Rules

### Catalog objects

- Frozen：identity、revision 单调性、ObjId 绑定、ordered pattern 首命中语义、capability 只能收窄、原始 `provider_model_id` 用于调用。
- Extensible：带缺省行为的 optional UI hints、诊断字段和新 pricing context。
- 禁止通用 `extra` 改变安全或调用语义；新增解释能力必须通过 `required_features` 协商。

### Provider Instance

- Frozen：实例名称是 Zone 内稳定主键；catalog 无权修改实例私有字段。
- Extensible：区域、账号、折扣等 pricing context。
- 凭据字段只能通过 locked value/credential reference 扩展。

### Inventory LKGS

- Frozen：实例主键、成功快照语义、摘要校验、catalog revision 绑定。
- Extensible：`snapshot_json` 内具有缺省行为的动态诊断字段。
- table 可添加带默认值的列；不改变现有列语义。

## 8. Query Patterns

| 查询 | 支持方式 | 频率 |
| --- | --- | --- |
| 按实例加载 LKGS | inventory table PK | 启动及 discovery 失败时，高 |
| discovery 成功原子替换实例 LKGS | inventory table PK upsert | 中 |
| catalog activation 后列出旧 revision 快照 | activation index | 每次 catalog 生效，低 |
| 清理长期不存在的实例快照 | updated index + system-config 实例集合 | 维护任务，低 |
| 按 catalog kind/id 读取对象 | manifest 内存索引 + ObjId 文件名 | 启动/更新，中 |
| Model Driver exact/pattern 匹配 | active catalog 构建内存索引 | 每次 discovery，重启重建 |
| pricing/provider rule 解析 | active catalog 构建内存索引 | 每次调用，高 |

不允许在每次模型调用时扫描 RDB 或 catalog 文件。active catalog 必须在 activation 切换后构建不可变内存 snapshot，并以一次指针替换原子生效。
