# AICC Model Driver 与逻辑模型 FS 冻结设计

状态：**Frozen / Beta 2.2**

冻结日期：2026-09-14

Schema：Model Driver `schema_version = 1`，system-config metadata envelope `schema_version = 1`。

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
| builtin 逻辑路径定义与 factory tree | 版本化静态数据 | `service/model_defaults.rs`，随二进制发布 |
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

构建顺序冻结为：

```text
builtin definitions
  -> inventory 注册 exact models
  -> Model Driver logical_mounts
  -> auto admission
  -> factory overlay
  -> system overlay
  -> user overlay
  -> session overlay
  -> item graph / fallback graph validation
```

当前生产装配至少启用 factory overlay 与 settings 中的 session overlay；`RegistryLayers` 已冻结 `factory/system/user/session` 四个插槽，后续接入 system/user 时不得改变层级顺序。

## 4. Schema Definitions

### 4.1 Object Type: Model Driver Catalog

Naming Convention：一个原厂 vendor 一个稳定 `model_driver_id` 和一份完整文档。builtin 文件名为 `<vendor>.model.json`；文件名不参与运行时 identity。

```json
{
  "format": "buckyos.aicc.model-driver-catalog",
  "schema_version": 1,
  "schema_revision": 1,
  "model_driver_id": "openai",
  "revision_seq": 1,
  "required_features": [],
  "models": [],
  "patterns": [],
  "defaults": {},
  "variants": [],
  "version_rules": []
}
```

语义优先级为 exact `models[].id`、有序 `patterns[].match`、`defaults`、保守 fallback。规则可声明：`model_driver`、`exclude`、`parameter_scale`、`api_types`、`logical_mounts`、`capabilities`、`canonical_fields`、`pricing`、调度提示与 `version_rules`。

Provider endpoint、认证、operation、渠道别名、渠道限制和渠道价格不属于 Model Driver，必须进入 Provider Profile/Rules。

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

variant 解析是 Provider-first：具体 provider model 匹配到 Provider Rules variant 时，以其完整集合为准；只有没有任何 Provider variant 命中时才使用 Model Driver variants，两边不静态 merge。

## 5. Schema Version

- Model Driver 初始 `schema_version` 为 `1`；`schema_revision` 表示同一 schema version 内已启用的字段能力，`revision_seq` 表示发布顺序。
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
llm.summary, llm.translate, llm.reason, llm.vision, llm.long, llm.fallback,
embedding.text, embedding.multimodal, rerank,
image.txt2img, image.img2img, image.inpaint, image.upscale, image.bg_remove,
vision.ocr, vision.caption, vision.detect, vision.segment,
audio.tts, audio.asr, audio.music, audio.enhance,
video.txt2video, video.img2video, video.video2video, video.extend, video.upscale,
agent_runtime.computer_use
```

具体模型家族挂点和权重属于可发布数据，不冻结为永久产品承诺；其当前值以 builtin metadata 和 `model_defaults.rs` 为准。

## 10. Validation Checklist

- [x] Durable 与 disposable 数据已分离。
- [x] 文件系统例外及其原因已说明。
- [x] Model Driver 与 overlay schema/版本已说明。
- [x] 来源优先级、原子发布、失败恢复已说明。
- [x] 身份、overlay 层级和查询模式已说明。
- [x] 核心结构化库存使用平台 RDB，不绑定数据库实现。
