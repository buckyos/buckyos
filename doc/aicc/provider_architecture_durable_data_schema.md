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
| 当前 metadata 文件集合 | NDN | Model Driver、Provider Rules、Pricing、Known Provider 的当前版本文件；下载、校验和替换由 NDN 保证 |
| metadata 目标序列 | NDN | `metadata_target_seq` 指向当前已替换文件版本，持续保留而非消费后清除 |
| Provider inventory LKGS | AICC RDB | 每个 Provider Instance 最近一次成功 discovery 并解析后的动态库存快照 |

Provider Instance 中不保存明文凭据；只保存 system-config 现有 locked value 或 credential reference。Metadata 文件替换和刷新不能修改 Provider Instance 私有配置。

### Disposable Data（可丢弃数据）

| 数据项 | 存储 | 重建方式 |
| --- | --- | --- |
| catalog staging、`.part` | AICC cache | 下次更新重新下载 |
| 已编译 exact/pattern 索引 | 内存 | 从 active catalog 重建 |
| adapter operation registry | 进程内静态注册 | 服务启动时从代码重建 |
| 当前 Provider health、队列和短期错误率 | 内存 | 运行时重新采集 |
| refresh 退避计数 | 内存 | 重启后重新开始 |
| Provider 库存刷新任务和停止事件通道 | 内存 | 实例进入运行状态时创建；停止、禁用、删除、替换或服务退出时发送 `Stop` 并等待任务循环优雅退出 |
| resolved call | 请求生命周期内存 | 每次调用重新解析 |

## 3. Storage Strategy

### 3.1 Provider Instance 配置

Provider Instance 配置是 Zone 级配置，继续存储在 system-config，由 control-panel 写入、AICC 只读。AICC 不复制实例配置到本地数据库，避免两个配置真相源。

Protocol Adapter 是随程序发布并注册的代码，不属于可云更新 catalog。运行时 registry descriptor 至少包含 `protocol_family_id`、`protocol_adapter_id`、接口代际/状态、支持的 operations，以及可选 `base_adapter_id`。`base_adapter_id` 声明语义复用关系，不规定继承、组合或委托的具体实现。

同一协议族的新旧 API 形态必须使用不同 Adapter ID，例如 `openai-responses` / `openai-chat-completions` 和 `gemini-interactions` / `gemini-generate-content`。官方 Known Provider 默认新接口；历史 Adapter 仅在具体派生 Provider 需要时按需注册。自定义 Provider 创建/更新时由接入测试按“新接口优先、已注册历史接口其次”解析 Adapter，用户不提供版本；resolved Adapter 保存到 Provider Instance，不能由运行时调用失败触发隐式切换。

### 3.2 NDN 当前 Metadata 与目标序列

NDN 管理当前 metadata 文件集合，负责版本发现、下载、校验和替换。AICC 不规定其 index、manifest、ObjId、缓存目录、水位或回滚布局，也不持久化 activation。

文件替换成功后，NDN 推进持久的 `metadata_target_seq`。每个 Provider inventory 保存 `metadata_applied_seq`；下一次推理前或任一 Provider Instance 定时库存刷新时，AICC 统一收敛所有序列不一致的 Provider，不能只处理当前请求或当前 Provider。每个 Provider 重建前临时捕获 `metadata_updating_seq`，成功提交 inventory 后才把 applied seq 更新为该值。

### 3.3 Provider inventory LKGS

inventory LKGS 是按实例查询和替换的结构化状态，必须使用平台提供的 RDB instance，不绑定具体数据库后端。每个 Provider Instance 只保留一份已验证的最近成功快照；历史 inventory 不作为审计日志长期保存。

inventory LKGS 的生命周期与刷新任务分离。停止 Provider 不删除 LKGS，但必须先向刷新任务循环发送 `Stop` 并等待优雅退出；任务退出后不得再写 inventory 或 health。重新启用实例时基于 LKGS 与当前 metadata 目标序列决定只探测还是重建。

## 4. Schema Definitions

### 4.1 NDN-managed Metadata File Set

AICC 只约束各 metadata 文件被加载后的业务 schema，不定义 NDN 的发布对象 schema。文件版本、集合完整性、可信性、下载和替换均由 NDN 保证；若保证不足，应向 NDN 提交 bug。

### 4.2 Object Type: Model Driver Catalog

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

ModelRule 只允许模型技术字段：`id/pattern`、`parameter_scale`、`api_types`、`logical_mounts`、`capabilities`、`quality_score`、`version_rules` 引用和可选保守默认价格。禁止 endpoint、认证、protocol adapter、operation、Provider 请求参数、availability、实例健康状态和对象内嵌签名。Catalog 文件真实性与完整性由 NDN 文件交付契约保证，AICC 不重复校验。

### 4.3 Object Type: Provider Rules Catalog

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

### 4.4 Object Type: Pricing Catalog

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

### 4.5 Object Type: Known Provider Catalog

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

Known Provider 可以为 SN 指定 `protocol_adapter_id: "sn-openai"`，不能直接填 OpenAI 官方 Adapter。registry 中 `sn-openai.protocol_family_id = "openai"`、`sn-openai.base_adapter_id = "openai-responses"`，从而保留独立身份和从 SN 到特定 OpenAI API 代际的单向依赖。

### 4.6 External Object: Provider Instance Config

Description：system-config 中由用户管理的实例私有配置。

Content Schema：

- `provider_instance_name: string`，Zone 内唯一且不可由 catalog 更新修改。
- `provider_profile_id: string`，专用 Provider 或 `custom`。
- `protocol_adapter_id: string`，必须来自运行时注册表。
- `endpoint: string`。
- `credential_ref/locked credential fields`。
- `auth`：认证模式及其私有参数。SN 至少允许互斥的 `api_key` 和 `dynamic_login`；动态 token 只保存在运行时凭据缓存。
- 可选 `region/account/pricing_context`。
- 可选 `provider_rules_id` 和实例级 rules/pricing override。

### 4.7 Table: aicc_provider_inventory_lkgs

Description：每个 Provider Instance 最近一次成功 discovery 后的已验证 inventory。

| Column | Type | Nullable | Default | Description |
| --- | --- | --- | --- | --- |
| provider_instance_name | TEXT PK | NO | | Provider Instance ID |
| schema_version | INTEGER | NO | 1 | 行中 snapshot JSON 的 schema major |
| provider_profile_id | TEXT | NO | | 生成快照时使用的 Provider profile |
| protocol_adapter_id | TEXT | NO | | 生成快照时使用的 adapter |
| provider_model_list_fingerprint | TEXT | NO | | 最近一次 discovery model 列表摘要，只用于变化判断 |
| metadata_applied_seq | INTEGER | NO | | 该库存已经正式应用的 NDN metadata 目标序列 |
| inventory_revision | TEXT | YES | | Provider discovery revision |
| discovered_at_ms | INTEGER | NO | | 最近成功 discovery 时间 |
| snapshot_json | TEXT | NO | | 完整 `ProviderInventorySnapshot` JSON |
| snapshot_sha256 | TEXT | NO | | snapshot_json 的 SHA-256 |
| created_at_ms | INTEGER | NO | | 首次保存时间 |
| updated_at_ms | INTEGER | NO | | 最近原子替换时间 |

Indexes：

- `idx_aicc_provider_inventory_lkgs_updated` ON `aicc_provider_inventory_lkgs(updated_at_ms)`：维护和诊断。
- `idx_aicc_provider_inventory_lkgs_metadata` ON `aicc_provider_inventory_lkgs(metadata_applied_seq)`：全局收敛时统一定位序列落后的库存。

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
| 旧 driver metadata v1 cache | Ignore；不读取、不迁移 |
| Provider catalog v2 objects/activation | Ignore；AICC 不再维护该存储结构 |
| inventory LKGS | Rebuild；schema 不匹配或摘要无效时删除该实例行并重新 discovery |
| Provider Instance config | No-compat；control-panel 与 AICC 同步切换到新字段 |
| staging/运行时索引 | 旧 staging 忽略；运行时索引从 NDN 当前文件重建 |

inventory 行迁移或重建失败不能阻止 AICC 使用已验证的内置 default inventory；不得把无效旧行标记为最新成功快照。

## 7. Extensibility Rules

### Catalog objects

- Frozen：业务 identity、ordered pattern 首命中语义、capability 只能收窄、原始 `provider_model_id` 用于调用。文件 revision、ObjId 和可信交付属于 NDN。
- Extensible：带缺省行为的 optional UI hints、诊断字段和新 pricing context。
- 禁止通用 `extra` 改变安全或调用语义；新增解释能力必须提升 AICC 支持的 metadata schema。

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
| 目标序列触发全局 inventory 收敛 | `metadata_target_seq` + Provider `metadata_applied_seq` | 推理前或 Provider 定时库存刷新时，低 |
| 清理长期不存在的实例快照 | updated index + system-config 实例集合 | 维护任务，低 |
| 按 catalog kind/id 读取对象 | 当前 metadata 内存 snapshot | 启动/全局刷新，中 |
| Model Driver exact/pattern 匹配 | active catalog 构建内存索引 | 每次 discovery，重启重建 |
| pricing/provider rule 解析 | active catalog 构建内存索引 | 每次调用，高 |

目标序列与所有 Provider applied seq 相同时，不允许在每次模型调用中扫描 metadata 文件。推理前或 Provider 定时库存刷新发现序列不一致时，必须捕获目标序列、加载对应完整 metadata snapshot，并统一收敛所有落后库存；不得按当前调用或 Provider 局部处理。定时 discovery 的 model 列表未变化且序列相同时只探测，不写 inventory。
