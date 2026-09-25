# AICC Model Driver 与逻辑模型 FS 冻结设计

状态：**Frozen / Beta 2.2**

冻结日期：2026-09-14

LLM 实现更新：2026-09-25，Model Driver v2 与规格/家族树已实现。当前契约见 §3.3、[Metadata Schema](driver_metadata_schema.md) 和 [实现报告](model_driver_v2_implementation.md)。

当前 Schema：Model Driver `schema_version = 1`，system-config metadata envelope `schema_version = 1`；新的 Model Driver 结构计划使用 v2，不改变 envelope 版本。

价格边界修订：2026-09-25，所有 Model Driver（含非 LLM 模型）不再声明 `model_pricing` 或兜底价格。builtin 配置、运行时 schema 和 Model Driver 价格 fallback 均已删除；Provider 价格与实际结算保持原边界。

## 1. Overview

服务：AICC。

Model Driver 把原厂模型身份转换为稳定的 AICC 语义；逻辑模型 FS 是以 `.` 分隔路径表示的内存虚拟目录树，把用途路径映射到模型家族或 exact model。它不是 `$BUCKYOS_ROOT` 下的真实文件系统。Provider discovery 提供渠道实际存在的模型，Provider Rules 先把渠道模型归一到 `origin_model_id`，Model Driver 再赋予 API type、能力、逻辑挂点、版本和 variant 语义。

公共路由协议见 [Trait 与 Protocol 冻结设计](frozen_trait_protocol.md)，字段全集见
[Model Driver Metadata Schema](driver_metadata_schema.md) 和 [统一匹配规则](match_rule.md)。

## 2. Data Classification

| 数据项 | 分类 | 所有者与位置 |
| --- | --- | --- |
| builtin Model Driver 源文件 | 版本化静态数据 | `src/frame/aicc/driver_metadata/models/*.model.json`，构建时嵌入 AICC 二进制 |
| local Model Driver override | Durable | `$BUCKYOS_ROOT/etc/aicc/driver_metadata/local/models/*.json`，本机管理员拥有 |
| cloud Model Driver 集合 | Durable | NDN 更新链路拥有；AICC 只消费当前完整发布 |
| system-config Model Driver 集合 | Durable | `services/aicc/driver_metadata.model_drivers`，最高优先级 |
| builtin 功能目录与功能到规格的偏好权重 | 版本化静态数据 | `service/model_defaults.rs`，随二进制发布 |
| LLM 规格、模型能力、规格归属与 effort | Model Driver 版本化数据 | 每个原厂一份 `models/*.model.json`；版本顺序从官方模型 ID 推导，与 inventory 分离 |
| system/user 逻辑树与 Provider 权重 | Durable | system-config AICC settings/路由配置，通过 CAS 更新 |
| request/session overlay | Disposable | 请求或 session 生命周期；含可选 TTL，不作为长期事实源 |
| 有效 `CatalogSnapshot` / `ModelRegistry` | Disposable | 进程内不可变 snapshot，可由上述来源和 inventory 重建 |
| Provider inventory LKGS | Durable、可重建 | 平台 RDB；schema 见 provider durable data 文档 |
| 路由候选、admission 结果、分数 | Disposable | 单次路由计算；审计结果另按 runtime durable schema 保存 |

## 3. Storage Strategy

### 3.1 Model Driver 来源

四层来源按每个 `(catalog_kind, catalog_id)` 独立选择完整文档：

```text
system-config > local filesystem > cloud/NDN > builtin
```

不同来源之间不做字段、数组或 rule merge。高优先级只遮蔽同一身份，不遮蔽低优先级的其它身份。有效集合全部验证成功后才发布新的不可变 `CatalogSnapshot`。

local 直接使用文件系统是明确例外：它是管理员离线 override/诊断入口，不是运行时核心模型；核心运行态由 snapshot 管理。只枚举三个固定目录的直接 `*.json` 文件，拒绝嵌套目录、symlink 和其它文件。完整树读取两次且内容一致才接受，revision 为排序后相对路径与内容的 SHA-256。

system-config 使用一个原子 value，避免读取到多文件的混合版本。结构化 Provider inventory 使用平台 RDB，不绑定具体数据库后端。

### 3.2 逻辑模型 FS

逻辑 FS 只存在于 `ModelRegistry`。路径用点分隔，例如 `llm.plan`、`image.txt2img`；`@` 仅属于 exact model，逻辑路径禁止包含 `@`。节点由 `LogicalModelDefinition` 定义 API type、minimum line、disable line、mount mode、scheduler profile、fallback、policy 和用户可见 tier。

当前生产装配至少启用 factory overlay 与 settings 中的 session overlay；`RegistryLayers` 已冻结 `factory/system/user/session` 四个插槽，后续接入 system/user 时不得改变层级顺序。

### 3.3 LLM 厂商规格与模型家族

复用 catalog、`LogicalModelDefinition`、带权重 item 和分层 overlay，规格从 metadata 静态创建，家族随有效库存生成。旧 current winner、版本挂点和 Model Driver variant 参数模板已删除。

职责固定为：

- Model Driver 声明厂商规格 `specs`，AICC 据此创建 `llm.{spec.id}`。例如 OpenAI driver 声明 `gpt-nano/mini/standard/pro/max` 五个通用规格，另有专用 `gpt-codex`；规格 ID 的产品线前缀不要求等于 driver ID `openai`。
- 配置主体 `models[]` 定义官方 `origin_model_id`、API 类型和能力，并在 `llm` 中明确唯一规格、固定 `effort`、`default_effort`、`supported_efforts` 与稳定性。规格内版本顺序从官方模型 ID 推导，不逐模型填写 `version_order`。不按 `parameter_scale` 或名字前缀猜规格归属，不因思考强度变化将同一模型自动分入多个规格。
- 家族默认是 `llm.{归一化官方模型ID}`，例如 `gpt-5.6-sol` 对应 `llm.gpt-5-6-sol`。原始 ID 保留用于匹配和调用；逻辑段归一化后须检查重名。同一家族是多个物理 instance 的汇集处，Provider 渠道 ID 先归一到官方身份，再挂入同一家族。
- `llm.gpt-5-6-sol:high` 是家族的固定思考预设；其下引用 `gpt-5.6-sol:reasoning-high@provider-a` 等 exact model。`:high` 不是 `.high` 子目录，不能由请求改成其他强度。Provider 无法执行该预设时，该实例不成为此预设的候选。
- `supported_efforts` 是模型支持强度的唯一声明，AICC 据此派生思考 variant 身份；Model Driver 不再维护重复的 `variants` 模型列表或参数模板。标准参数转换属于 Protocol Adapter，渠道差异与限制属于 Provider Rules；没有适用转换的实例不能执行该预设。树只消费库存已明确提供的 variant；没有现成 Provider Rules 映射时无候选，新增标准转换仍属后续 Adapter 接入。
- 功能到规格的偏好权重继续由 `model_defaults.rs` 和显式 overlay 管理；规格到家族的关系从模型条目生成，不在两个地方重复维护。家族直选使用 metadata 声明的默认预设，默认 strict。

已归入规格的模型不再声明 `logical_mounts`，包括原先逐模型列出的 `vision.*`、`image.*`、`agent_runtime.*` 路径。模型定义负责官方身份、能力、规格与预设，功能路径及引用由通用逻辑树编排。`api_types` 是可执行能力约束，不能仅凭它或 LLM 规格归属自动接入所有非 LLM 任务；这些入口须单独配置树引用并检查 API 能力。builtin overlay 已显式引用所需规格，并在展开时检查非 LLM API。独立图片、音频、视频和 embedding 模型保留现有挂点。

LLM 装配顺序为：

```text
有效厂商 metadata + builtin 功能定义
  -> 声明规格目录（零 inventory 也存在）
  -> metadata 与有效 inventory 相交，注册 exact instances
  -> 动态家族 / 固定预设 / 规格到家族预设的版本引用
  -> factory 功能偏好 -> system -> user -> session overlay
  -> 规格引用、归属、预设、版本顺序与图校验
  -> 原子发布 ModelRegistry
```

对外结构为 `功能 -> 规格 -> 模型家族:固定预设 -> 物理 instance`。LLM 功能和规格不启用通用 Auto/Hybrid，不接受 inventory/`auto_mounts` 直接挂载。`llm` 只作命名空间，`llm.fallback` 默认空；任务、规格、家族禁止隐式 Parent fallback，显式回退仍保留原任务约束。

先过滤可执行候选，跳过空规格，再按功能权重选规格、在规格内按推导的版本值选最新合格稳定家族，最后调度物理实例。OpenAI 的 `5.6 -> 560`、`5.5 -> 550`、`6 -> 600`；同版本允许同值，并以归一化家族 ID 升序稳定排序，不依赖配置声明顺序。版本分段、扩展数字段和无版本模型的处理见 Metadata Schema，不把日期、参数量或产品后缀当版本号。实验版仅在无合格稳定版且策略允许时参与。两层顺序不相乘，Provider 数量不增加家族权重；规格耗尽再换下一个规格。

inventory 消失只清理动态家族/预设及其引用，保留空规格、功能节点及偏好权重。官方 Provider 停供时保留模型定义，其他 Provider 仍可提供同一家族。每个规格须被功能引用或声明 `direct_only`，每个有效 LLM 模型规则须归档到唯一规格；零库存也校验这些声明。字段、JSON 示例、迁移边界和验收项统一见 [Metadata Schema](driver_metadata_schema.md#llm-target-contract-vendor-specifications-and-model-families)。

## 4. Schema Definitions

### 4.1 Object Type: Model Driver Catalog

Naming Convention：一个原厂 vendor 一个稳定 `model_driver_id` 和一份完整文档。builtin 文件名为 `<vendor>.model.json`；文件名不参与运行时 identity。

当前 Model Driver 文档骨架如下；模型字段详见 [Metadata Schema](driver_metadata_schema.md)。

```json
{
  "format": "buckyos.aicc.model-driver-catalog",
  "schema_version": 2,
  "schema_revision": 0,
  "model_driver_id": "openai",
  "revision_seq": 1,
  "required_features": [],
  "models": [],
  "patterns": [],
  "defaults": {},
  "specs": []
}
```

语义优先级为 exact `models[].id`、有序 `patterns[].match`、`defaults`、保守 fallback。技术规则可声明：`model_driver`、`exclude`、`parameter_scale`、`api_types`、`logical_mounts`、`capabilities`、`canonical_fields`、调度提示与 `llm`。Model Driver 不声明价格；技术语义的保守 fallback 不包含价格或费用估值兜底。

Provider endpoint、认证、operation、渠道别名、渠道限制和渠道价格不属于 Model Driver，必须进入 Provider Profile/Rules。

价格取当前 Provider 的 discovery 数据或适用的 Provider Rules `model_pricing`，实际结算优先采用 Provider 响应的费用。没有可靠来源时保持 unknown，不按 0 或原厂报价补齐；官方直连渠道也不例外。删除的价格不能未经渠道及计费条件确认就搬到 Provider Rules。完整规则见 [Provider Schema §7](provider_profile_schema.md#7-价格优先级)。

### 4.2 Object Type: Logical Route Overlay

持久 overlay 的公共 schema 是 `AiccRouteOverlay`：

| 字段 | 语义 |
| --- | --- |
| `inherit` | 可选父配置引用 |
| `logical_tree` | 递归的 `AiccLogicalNodeOverlay` |
| `logical_profile(s)` / `active_logical_profile` | profile 组合与当前选择 |
| `global_exact_model_weights` | exact model 全局权重 |
| `provider_weights` | Provider Instance 权重 |
| `policy` | 锁定值、候选过滤和 scheduler 权重 |
| `revision` | 调试/发布 revision |
| `ttl_seconds` | 临时 overlay 生命周期提示 |

节点可提供 `items`、`item_overrides`、`exact_model_weights`、`disable_line`、`fallback`、`policy` 和子节点。`ModelItem` 固定为 `{ target, weight }`；target 可以是逻辑路径或 exact model。

### 4.3 Identity

```text
origin model       = <origin_model_id>
provider model     = 渠道实际调用 ID
model uid          = <origin_provider>/<origin_model_id>[/<variant>]
exact model        = <provider_model_id>[:<variant>]@<provider_instance_name>
logical model      = 点分隔用途路径，不含 @
```

LLM effort 身份由 `supported_efforts` 派生，与现有库存中的可用变体取交集。调用仅使用现有 Provider Rules 的参数映射；Model Driver 不再提供 variant 参数 fallback。没有映射的 effort 不声明可执行，标准 Adapter 转换另行接入。

## 5. Schema Version

- Model Driver 当前 `schema_version` 为 `2`，`schema_revision` 为 `0`；`schema_revision` 表示同一 schema version 内已启用的字段能力，`revision_seq` 表示发布顺序。
- system-config metadata envelope 初始 `schema_version` 为 `1`，版本保存在 value 顶层。
- 逻辑 overlay 的结构由 `buckyos-api` 公共 Rust DTO 版本和 system-config revision 共同管理，当前没有独立 `schema_version` 字段；这是冻结现状，新增持久结构版本时必须先补 version 字段。
- schema 结构变化递增 `schema_version`；兼容字段能力变化递增 `schema_revision`；内容发布只递增 `revision_seq`。

## 6. Upgrade Compatibility Strategy

Beta 2.2 采用 **No-compat**：旧 `provider_driver`、根级 `provider_options`、`origin_provider_aliases`、`origin_mappings`、`signature` 直接拒绝，不提供迁移 alias。

| 数据项 | 策略 |
| --- | --- |
| builtin metadata | Rebuild：升级二进制重新嵌入 |
| cloud metadata | Rebuild/replace：由 NDN 原子发布完整集合 |
| local metadata | No-compat：管理员按当前 schema 修正，解析失败不发布 |
| system-config metadata/overlay | No-compat：CAS 写入当前 schema，解析失败保留当前 runtime snapshot |
| ModelRegistry | Rebuild：每次有效来源或 inventory 变化时重建 |

任何加载、引用或图校验失败都必须 error-and-keep-current；不得发布部分 catalog 或部分逻辑树。

## 7. Extensibility Rules

冻结字段：身份字段、`format`、版本字段、exact/logical 名称格式、来源优先级、整文档 shadow、层级应用顺序、fallback 模式语义。

可扩展字段：新的 Model Driver rule 字段、capability key、canonical converter、logical node 属性和 scheduler profile；扩展必须通过显式 schema revision/version 与 `deny_unknown_fields` 校验上线，不能依靠任意 `extra` map 绕过评审。

`capabilities`、`provider_options` 和部分 attributes 虽为 JSON map，也只允许各自层的既定语义：Model Driver 不能借它们携带 endpoint/auth/operation，Provider Rules 不能借它们增加原厂固有能力。

## 8. Query Patterns

| 查询 | 支持结构 | 代价 |
| --- | --- | --- |
| 按 exact model 查模型 | `ModelRegistry.models` key | 对数级 |
| 按 logical path 展开候选 | `logical_nodes` + 已验证 item graph | 与访问节点/候选数相关 |
| 按 `(kind,id)` 选择 metadata | source resolver identity map | 每次 snapshot 构建一次 |
| 枚举模型与逻辑节点 | `models.list` 的 snapshot view | 全量枚举，仅管理面 |
| 按 Provider 重建可路由库存 | inventory snapshot + catalog indexes | refresh/reload 路径，不在请求热路径扫描文件 |

请求热路径禁止扫描 metadata 文件、访问 system-config 或重新解析 JSON。

## 9. 冻结逻辑目录

一级 API 目录冻结为：`llm`、`embedding`、`rerank`、`image`、`vision`、`audio`、`video`、`agent_runtime`。当前 builtin 用途节点包括：

```text
llm, llm.chat, llm.plan, llm.code, llm.swift, llm.summarize,
llm.translate, llm.vision, llm.fallback,
embedding.text, embedding.multimodal, rerank,
image.txt2img, image.img2img, image.inpaint, image.upscale, image.bg_remove,
vision.ocr, vision.caption, vision.detect, vision.segment,
audio.tts, audio.asr, audio.music, audio.enhance,
video.txt2video, video.img2video, video.video2video, video.extend, video.upscale,
agent_runtime.computer_use
```

具体模型家族挂点和权重属于可发布数据，不冻结为永久产品承诺；其当前实现值以 builtin metadata 和 `model_defaults.rs` 的代码为准。该文件头部契约已落实到 builtin definitions、overlay 和零 Provider 回归测试。

## 10. Validation Checklist

- [x] Durable 与 disposable 数据已分离。
- [x] 文件系统例外及其原因已说明。
- [x] Model Driver 与 overlay schema/版本已说明。
- [x] 来源优先级、原子发布、失败恢复已说明。
- [x] 身份、overlay 层级和查询模式已说明。
- [x] 核心结构化库存使用平台 RDB，不绑定数据库实现。
