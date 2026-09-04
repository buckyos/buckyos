# AICC Provider Architecture — Durable Data Schema

## 1. Overview

服务：AICC。

关联设计：

- GitHub Issue `buckyos/buckyos#579`
- [provider_profile_schema.md](provider_profile_schema.md)
- [driver_metadata_update_protocol.md](driver_metadata_update_protocol.md)
- [driver_metadata_update_storage.md](driver_metadata_update_storage.md)

本文定义 Model Driver catalog、Provider mapping/rules catalog、Known Provider catalog、Provider Instance 配置和 Provider Instance 级 inventory LKGS 的持久数据边界。渠道静态价格属于 Provider Rules，不单独建立 Pricing catalog。目标是让静态模型语义、渠道规则、动态可用性和实例私有配置分别拥有唯一真相源，并能在服务重启、catalog 更新失败或 discovery 失败后确定性恢复。

## 2. Data Classification

### Durable Data（持久数据）

| 数据项 | 所有者 | 说明 |
| --- | --- | --- |
| Provider Instance 配置 | system-config | 用户配置的实例名称、Provider profile、protocol adapter、`base_url`、区域及凭据引用 |
| builtin metadata 文件 | BuckyOS 发布包 | Model Driver、Provider Rules、Known Provider 的最低优先级只读来源 |
| 当前 cloud metadata 文件集合 | NDN | 当前云来源版本；下载、校验和替换由 NDN 保证 |
| local metadata 文件 | 本机管理员 | 高于 cloud、低于 system-config 的持久来源 |
| system-config metadata 来源 | system-config | 最高优先级的 Zone 配置来源 |
| metadata 发布选择与目标序列 | NDN 更新链路 | 云端按客户端版本/通道/灰度分组选择兼容发布；本机 `metadata_target_seq` 等于严格递增的 manifest `revision_seq`，持续保留且不允许回退 |
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

Provider Instance 配置是 Zone 级配置，继续存储在 system-config。AICC Runtime 只读取 settings；AICC 管理 API 是受控写入 facade，使用当前 RPC 调用者 token 和 settings revision，通过 `SystemConfigClient::exec_tx` 做 CAS 更新。前端不得直接写 system-config。AICC 不复制实例配置到本地数据库，system-config 始终是唯一配置真相源。

Protocol Adapter 是随程序发布并注册的代码，不属于可云更新 catalog。运行时 registry descriptor 至少包含 `protocol_family_id`、`protocol_adapter_id`、接口代际/状态、支持的 operations，以及可选 `base_adapter_id`。`base_adapter_id` 声明语义复用关系，不规定继承、组合或委托的具体实现。

同一协议族的新旧 API 形态必须使用不同 Adapter ID，例如 `openai-responses` / `openai-chat-completions` 和 `gemini-interactions` / `gemini-generate-content`。官方 Known Provider 默认新接口；历史 Adapter 只在首个实际 Provider 需求出现时按需实现和注册，但注册后是协议族级共享能力，不归触发它的派生 Provider 私有。其它 Provider 使用相同历史 API 代际时直接引用该 Adapter；存在渠道差异时，其派生 Adapter 通过 `base_adapter_id` 复用它。自定义 Provider 创建/更新时由接入测试按“新接口优先、已注册历史接口其次”解析 Adapter，用户不提供版本；resolved Adapter 保存到 Provider Instance，不能由运行时调用失败触发隐式切换。

### 3.2 Metadata 来源、NDN 云来源与目标序列

NDN 只管理当前 cloud metadata 文件集合，负责版本发现、下载、校验和替换。Index、manifest、三类 catalog 发布路径及必要字段由更新协议固定；具体 ObjId 表达、下载缓存、水位存储和文件替换布局属于 NDN 实现，AICC 不持久化 activation。

AICC 加载 `builtin`、`cloud`、`local`、`system-config` 四个独立来源，并按 `(catalog_kind, catalog_id)` 以 `system-config > local > cloud > builtin` 逐项选择。每个身份只启用最高优先级来源的完整文件，不跨来源合并字段、数组或规则。高优先级来源只遮蔽其中实际存在的身份，因此 cloud 只有 OpenAI 更新时，builtin MiniMax 仍参与最终有效集合。选择完成后才校验整个有效集合并构建不可变 snapshot。

上述四层加载和选择全部属于 metadata source manager。builtin JSON 由该管理模块从 `src/frame/aicc/driver_metadata/` 统一编译嵌入，不存在生产运行时 builtin 目录；其它三层仅由其管理入口和统一 loader 接触具体路径或 key。Provider、Service、Routing 和 Execution 只能使用 metadata source manager 已发布的有效 snapshot，不得自行加载、缓存或选择来源。

每个 metadata manifest 声明严格递增且不可复用的 `revision_seq`、兼容客户端版本范围和 required features。云更新服务可以给不同客户端版本配置不同发布版本；NDN 更新链路只接受与本机客户端兼容且序列高于已接受水位的发布，替换成功后令持久的 `metadata_target_seq = manifest.revision_seq`。回退必须把旧内容重新发布为更高序列的新版本，不能降低本机水位。

每个 Provider inventory 保存 `metadata_applied_seq`；下一次推理前或任一 Provider Instance 定时库存刷新时，AICC 统一收敛所有序列不一致的 Provider，不能只处理当前请求或当前 Provider。每个 Provider 重建前临时捕获 `metadata_updating_seq`，成功提交 inventory 后才把 applied seq 更新为该值。

### 3.3 Provider inventory LKGS

inventory LKGS 是按实例查询和替换的结构化状态，必须使用平台提供的 RDB instance，不绑定具体数据库后端。每个 Provider Instance 只保留一份已验证的最近成功快照；历史 inventory 不作为审计日志长期保存。

inventory LKGS 的生命周期与刷新任务分离。停止 Provider 不删除 LKGS，但必须先向刷新任务循环发送 `Stop` 并等待优雅退出；任务退出后不得再写 inventory 或 health。重新启用实例时基于 LKGS 与当前 metadata 目标序列决定只探测还是重建。

## 4. Schema Definitions

### 4.1 Metadata File Sources

Cloud 来源的 index、manifest 和 catalog 路径由 [driver_metadata_update_protocol.md](driver_metadata_update_protocol.md) 定义；本节只展开各来源 catalog 文件被选择后的业务 schema。Cloud 文件版本、来源可信性、下载和替换由 NDN 保证；builtin 由 metadata source manager 编译进 AICC，local 由本机管理员管理，system-config 由配置服务管理。metadata source manager 负责按身份整文件择优，并校验最终有效集合的 schema、唯一性和跨 catalog 引用。

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
- `patterns: ModelRule[]`，每项使用统一 `MatchRule`，有序、首个命中生效
- `defaults: ModelSemanticDefaults`
- `variants: ModelVariant[]`
- `version_rules: VersionRule[]`

ModelRule 只允许模型技术字段：`id/match`、`parameter_scale`、`api_types`、`logical_mounts`、`capabilities`、`quality_score`、`version_rules` 引用和可选保守默认价格。`match` 遵循 [match_rule.md](match_rule.md)，普通规则直接使用 wildcard 字符串。禁止 endpoint、认证、protocol adapter、operation、Provider 请求参数、availability、实例健康状态和对象内嵌签名。Catalog 文件真实性与完整性由 NDN 文件交付契约保证，AICC 不重复校验。

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
- `patterns: ProviderModelRule[]`，每项使用统一 `MatchRule`，有序、首个命中生效
- `variants: ProviderVariantRule[]`

ProviderModelRule 可包含 `match`、`exclude`、`operations`、`provider_options`、`request_rules`、`pricing`、`remove_api_types`、`remove_features`、`estimated_latency_ms`、`latency_class`、`cost_class`。`match` 通常是匹配 `provider_model_id` 的 wildcard 字符串，需要联合 `origin_model_id`、Model Driver、variant 或 API type 时才使用对象。request/pricing 条件也复用同一 `MatchRule`。`pricing` 直接保存该渠道模型的静态价格和条件计价规则。配置只能收窄 Model Driver 能力。

`metadata_drivers` 缺失表示搜索系统当前安装的全部 Model Driver；显式空数组表示不匹配任何 Model Driver。每个官方支持的 Provider 厂商（包括内置专用 Provider）都必须有独立文件。

空对象 `{}` 是未被官方 catalog 收录的 `custom` Provider 的标准运行时规则：协议族由用户明确选择并在接入测试后解析为 Adapter；模型解析保留原始 `provider_model_id`，在全部 Model Driver 中要求唯一匹配，不执行 prefix/suffix stripping、vendor alias 或其它渠道映射。该空规则属于 Provider Instance 的解析结果，不冒充 NDN 发布的官方 Provider Rules catalog。

Provider Rules 是模型映射、operation、请求参数、能力收窄、静态价格及可声明 dialect 差异的唯一持久真相源。Rust builtin 或 dialect 不得构造另一份生产规则；遇到现有 schema 无法表达的差异时，应先评审有界 schema 扩展，只有无法安全声明化的执行逻辑才进入代码。

### 4.4 Object Type: Known Provider Catalog

Description：管理 UI 使用的已知服务商列表。

Naming Convention：`v2/known-providers/<catalog_id>-<revision_seq>.json`。

Content Format：UTF-8 JSON。

Content Schema：

- `format: "buckyos.aicc.known-provider-catalog"`
- `schema_version: 1`
- `schema_revision: u32`
- `revision_seq: u64`
- `catalog_id: string`
- `providers[]`：
  - `provider_profile_id`、`display_name`；
  - `base_url`：默认 base URL，可包含 `{region}`、`{workspace}`、`{account}` 占位符；
  - `protocol_adapter_id`；
  - `provider_rules_id`：正式 Provider configuration 必须提供，且被引用 Rules 的 `provider_profile_id` 必须与本项一致；
  - `credential`：typed 默认凭据描述，`kind` 为 `bearer`、`named_header`、`fal_key` 或 `glm_jwt`；`named_header` 必须同时提供非空 `header_name`；
  - 可选 `credential_variants[]`：同结构的实例级可选凭据；kind 不得与默认凭据或其它变体重复，实例通过 `auth.credential_kind` 显式选择，省略时使用默认凭据；
  - `connection.region/workspace/account`：每项均包含 `mode: unsupported|optional|required`，可选 `default_value`、`allowed_values`；unsupported 字段不得携带默认值或允许值，默认值必须属于非空的 allowed values；
  - 可选 `connection.region_base_urls`：从已声明的 `allowed_values` region 到默认 base URL 的 typed 映射；实例显式 `base_url` 优先于该映射；
  - 可选 `ui_hints`：只承载展示提示，不是 Provider Profile、连接合同、认证或 Rules identity 的配置来源。

`credential_variants` 与 `connection.region_base_urls` 从 Known Provider `schema_revision: 1` 起可用；revision 0 文档携带这些字段必须拒绝。

`CatalogSnapshot::resolve_provider_configuration(provider_profile_id)` 把以上数据解析为 `ResolvedProviderConfiguration`。返回结果包含默认 credential、credential variants、typed connection schema、默认及区域 base URL、Adapter ID 和 Rules ID。Known Provider 不存在、Rules 引用缺失、Rules 不存在、identity 不一致或 typed 字段无效时必须 fail closed，调用方不得回退到 `ui_hints` 或 Rust builtin metadata helper。

该对象只声明可直接生成 `ProviderProfile` 和 `ProviderConnectionContract` 的静态配置。Provider 行为 registry 负责执行 credential variant 选择和 region URL 选择，并注册 discovery、refresh、default inventory、SN dynamic login 等不可声明化行为；registry 必须基于同 generation snapshot 的 resolved configuration 装配，不能维护另一份默认配置。

该 catalog 只提供默认值。保存 Provider Instance 前必须让用户看到并允许修正协议和 `base_url`，并执行连接与协议验证。

Known Provider 的显示信息、默认 `base_url`、默认 Adapter 和 UI hints 不得由 Rust builtin 维护第二份生产副本；builtin 只注册相应行为实现并消费 catalog。

Known Provider 可以为 SN 指定 `protocol_adapter_id: "sn-openai"`，不能直接填 OpenAI 官方 Adapter。registry 中 `sn-openai.protocol_family_id = "openai"`、`sn-openai.base_adapter_id = "openai-responses"`，从而保留独立身份和从 SN 到特定 OpenAI API 代际的单向依赖。

### 4.5 External Object: Provider Instance Config

Description：system-config 中由用户管理的实例私有配置。

Content Schema：

- `provider_instance_name: string`，Zone 内唯一且不可由 catalog 更新修改。
- `provider_profile_id: string`，专用 Provider 或 `custom`。
- `protocol_adapter_id: string`，必须来自运行时注册表。
- `base_url: string`，Protocol Adapter 在此基础上构造具体 operation URL。
- `credential_ref/locked credential fields`。
- `auth`：认证模式及其私有参数。SN 至少允许互斥的 `api_key` 和 `dynamic_login`；动态 token 只保存在运行时凭据缓存。
- 可选 `region/account`。
- 可选 `provider_rules_id` 和实例级非价格 rules override。

### 4.6 Table: aicc_provider_inventory_lkgs

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
- 三类 catalog 对象的初始 `schema_version` 均为 `1`；Beta 2.2 尚未发布，不接受缺少 typed Provider configuration 的旧 Known Provider 对象。
- inventory LKGS table row `schema_version`：`1`。
- Provider Instance config schema 由 system-config 对应 settings 文档维护，本架构切换后使用新字段，不读取旧 `provider_driver` 兼容别名。

`schema_revision` 只增加具有明确缺省行为的可选字段；解释语义不兼容时提升 `schema_version`。catalog protocol 或本地原子提交语义变化时提升目录/protocol major。

## 6. Upgrade Compatibility Strategy

当前版本为 beta 2.2 breaking change，以下 No-compat 只针对本机旧存储和配置迁移；云更新仍必须为当前受支持的不同客户端版本投放各自兼容且不回退的 metadata：

| 数据项 | 策略 |
| --- | --- |
| 旧 driver metadata v1 cache | Ignore；不读取、不迁移 |
| Provider catalog v2 objects/activation | Ignore；AICC 不再维护该存储结构 |
| inventory LKGS | Rebuild；schema 不匹配或摘要无效时删除该实例行并重新 discovery |
| Provider Instance config | No-compat；control-panel 与 AICC 同步切换到新字段 |
| staging/运行时索引 | 旧 staging 忽略；运行时索引从四个来源逐身份择优后的有效文件重建 |

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
| Model Driver exact/pattern 匹配 | effective catalog 构建内存索引 | 每次 discovery，重启重建 |
| pricing/provider rule 解析 | effective catalog 构建内存索引 | 每次调用，高 |

云目标序列与所有 Provider applied seq 相同时，不允许在每次模型调用中扫描 metadata 文件。推理前或 Provider 定时库存刷新发现序列不一致时，必须捕获云目标序列、按 `system-config > local > cloud > builtin` 逐身份选择并加载对应完整有效 metadata snapshot，再统一收敛所有落后库存；不得按当前调用或 Provider 局部处理。local/system-config 变化由 reload 路径发布新 RuntimeSnapshot 并触发相应 inventory 重建。定时 discovery 的 model 列表未变化且序列相同时只探测，不写 inventory。
