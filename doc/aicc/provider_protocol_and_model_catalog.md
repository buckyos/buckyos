# AICC Provider 协议与模型适配设计

状态：设计草案，尚未实现
基线：`beta2.2` / `e691cd84334f9d432ac21148a3b7f1a7c7e0e179`
问题来源：PR #526（merge commit `44bdd16e0`）

## 1. 核心模型

一个 provider instance 由两个可复用定义和一组实例私有配置组合而成：

```text
ProviderInstance
  ├─ api_protocol_id       -> 程序实现的 API 访问协议
  ├─ provider_driver       -> 模型适配方案 JSON 的现有唯一 ID
  └─ instance settings     -> 实例名、访问 URL、凭据等私有配置
```

- API 协议决定怎样访问 provider，例如请求路径、认证头、请求/响应转换和模型发现格式。
- driver metadata 用 provider 返回的渠道模型名匹配 exact、pattern、defaults、version rules 和 variants，并同时把渠道模型名映射为原始模型引用，最终解析出 AICC inventory，包括 API type、capability、mount、价格和 origin。
- provider instance 只组合前两者并提供连接信息，不拥有或覆盖 driver metadata 内容。

OpenAI 和 OpenRouter 可以使用同一个 API 协议实现，但选择不同的 driver metadata。新增一个使用现有协议的 provider 时，只需增加 metadata JSON，并在构造实例时选择它，不应修改任何按 provider 名称判断的 Rust 代码。

## 2. 名词与字段

| 概念 | 内部字段 | 示例 | 所有者 |
| --- | --- | --- | --- |
| API 接口协议 | `api_protocol_id` | `openai`, `anthropic`, `gemini` | 程序 |
| Driver Metadata ID | `provider_driver` | `openai`, `openrouter` | metadata catalog |
| Driver Metadata 展示名 | `display_name` | `OpenAI`, `OpenRouter` | metadata catalog |
| Provider Instance 名称 | `provider_instance_name` | `openrouter-main` | 实例配置 |
| 访问 URL | `base_url` | `https://openrouter.ai/api/v1` | 实例配置 |
| 凭据与认证参数 | `credentials` / `auth` | token | 实例配置与 secret store |

`api_protocol_id` 和 `provider_driver` 是正交字段，不能再从实例名、URL 或彼此推断。

继续使用现有 `provider_driver` 表示实例选择的 metadata identity，不再发明新的 ID 字段。它是程序识别的规范机器 ID，必须使用全小写；面向人类、可保留品牌大小写的名称使用可选的 `display_name`。`display_name` 缺省或为空时，展示层复用 `provider_driver`，该 fallback 不参与程序识别。原始模型 identity 继续使用现有 `model_driver` 和 `origin_model_id`。需要消除的是使用 `provider_driver` 选择 API 协议、模型分类或计费行为，而不是重命名这些已有字段。

## 3. UI 名称

UI 不直接展示 “Driver Metadata”。推荐名称：

- 中文：**模型适配方案**
- 英文：**Model Adaptation Profile**

“模型适配”表示它负责把 provider 的渠道模型转换为 AICC 的统一模型描述，覆盖名称匹配、来源映射、准入、能力、API type、mount、价格、版本和 variant，而不只是生成模型目录。“方案”表示它是一份可以安装、更新并被多个 provider instance 复用的完整配置。

metadata 文档增加面向 UI 的可选字段：

```json
{
  "provider_driver": "openrouter",
  "display_name": "OpenRouter 模型适配方案",
  "description": "解析 OpenRouter 模型 ID、能力、价格和模型来源"
}
```

自定义 provider 表单建议展示：

1. 实例名称；
2. API 协议下拉框；
3. 访问地址；
4. API Key；
5. 模型适配方案下拉框。

两个下拉框都不允许自由输入 ID。API 协议选项来自程序已经实现的 `XxxProvider` 对象列表；模型适配方案选项来自当前已安装并验证通过的 metadata catalog，选项展示 `display_name`，缺省时展示 `provider_driver`，提交值始终分别为 `api_protocol_id` 和 `provider_driver`。需要使用新的 metadata 时，必须先将其安装或发布到 catalog，不能在创建 instance 时临时输入一个未知 ID。

内置 provider 不需要用户选择，由程序在构造内置实例时显式指定 `api_protocol_id` 和 `provider_driver`。

## 4. 配置所有权和覆盖边界

| 数据 | 内置默认 | 云更新可覆盖 | Provider instance 可覆盖 |
| --- | --- | --- | --- |
| API 协议实现 | 程序 | 否 | 只能选择程序已支持的协议 |
| Driver metadata JSON | 内置 catalog | 同 ID 的新版本可以 | 否 |
| 实例名称 | 实例配置 | 否 | 是 |
| 访问 URL | 实例配置 | 否 | 是 |
| 凭据、auth 参数 | 实例配置/secret store | 否 | 是 |
| 计费或产品策略 | 受信任程序配置 | 否 | 按权限控制，不属于 metadata |

云更新以 `provider_driver` 为键发布新版本，可以替换同 ID 的旧内置 JSON。它不能修改任何 provider instance，也不能通过 metadata 改写 URL、认证、凭据、协议或实例名称。

provider instance 配置中禁止出现 model rules、capabilities、prices、mounts、origin mappings、variants 等 metadata patch。需要调整这些内容时，必须创建或更新一个有独立 ID 和 revision 的完整 metadata 文档。

当前 local/system-config metadata override 若继续保留，也必须是 catalog 级、按 metadata ID 生效的完整文档，不能成为某个实例的私有 patch。其与云版本的优先级应由 metadata update 规范统一定义。

## 5. 程序内置的 API 协议实现

API 协议不需要数据驱动的 registry。它就是程序已经实现的 `XxxProvider` 对象列表，例如 OpenAI、Anthropic 和 Gemini 对应的 Provider 实现。

`api_protocol_id` 只用于从这组编译期已知的实现中选择一个对象。用 enum 或集中 `match` 构造对应 `XxxProvider` 都可以；这种分支表达的是程序实际支持哪些 API 协议，是必要的协议分发，不是需要消除的 provider 名称硬编码。

每个 `XxxProvider` 实现负责：

- endpoint 路径和 HTTP 方法；
- 请求 lowering 与响应 parsing；
- auth header 构造；
- `/models` 或等价模型发现协议；
- provider-native state 的保存与恢复；
- 协议实际支持的 AICC API types。

协议分发代码只认识程序支持的 API 协议，不认识 `openrouter` 等产品/provider 或 metadata 名称。

新增一种真正的新 API 协议需要实现新的 `XxxProvider`，并将它加入程序支持的对象列表，这是必要的代码变更；使用已有协议的新 provider 不需要代码变更。

### 5.1 薄协议适配器边界

`XxxProvider` 是 wire protocol adapter，不是同名厂商模型的语义容器。文件名为 `claude.rs` 或 `gemini.rs`，只表示它实现 Anthropic Messages 或 Gemini API；不能据此把 Claude、Gemini 模型的能力、价格和版本规则固化在该文件中。反过来，通过 OpenRouter 提供的 Claude/Gemini 模型仍由 `OpenAIProvider` 执行，因为当前 instance 选择的是 OpenAI-compatible API 协议，不能根据 `model_driver` 或 `origin_model_id` 改派到 `ClaudeProvider` 或 `GeminiProvider`。

职责边界如下：

| 内容 | 所有者 |
| --- | --- |
| URL path、HTTP method、auth header、请求/响应/stream 转换、错误解析 | API 协议实现 |
| 协议可以执行哪些 AICC API types、协议字段如何序列化 | API 协议实现 |
| 默认模型列表、渠道模型准入、API type、capability、feature、context/output limit、mount、价格、版本、variant、参数约束 | 当前 instance 选择的 metadata |
| 内置 endpoint、凭据引用 | 内置 provider instance preset |
| 远端 discovery 明确返回的 methods、deprecated、availability | 协议读取的动态事实，与 metadata 结果做限制性交集 |

协议实现不得包含按模型家族、厂商品牌或 `provider_driver` 选择语义的分支。它只消费 resolver 生成的 `ModelMetadata`，并按已经选择的 wire protocol 执行请求。metadata 只声明结构化语义，不允许携带脚本、HTTP header 模板或任意请求转换逻辑；若现有协议无法表达一个 provider 的 wire 行为，应实现新的 `XxxProvider`，而不是把 metadata 扩展成可执行 DSL。

OpenRouter 多原厂模型的解析示例：

```text
OpenRouter /models 返回 anthropic/claude-... 或 google/gemini-...
    -> openrouter.json 按完整 provider_model_id 匹配渠道规则
    -> origin mapping 生成现有 model_driver / origin_model_id
    -> openrouter.json 产出 capability / api_types / mounts / pricing / variants
    -> OpenAIProvider 按 OpenAI-compatible wire protocol 发起调用
```

这里 `openrouter.json` 是渠道语义的唯一真相源。即使模型来源是 Anthropic 或 Google，也不读取 `claude.rs`、`gemini.rs` 中的模型判断，不切换到 `claude.json` 或 `gemini.json`，也不继承这些文档的价格。需要共享维护经验时可以由 metadata 生成工具在发布前复用数据，但运行时文档仍保持独立，避免一个原厂 metadata 更新无意改变 OpenRouter inventory。

## 6. Driver Metadata Catalog

### 6.1 唯一身份

`driver_metadata/*.json` 中每个文档继续使用现有 `provider_driver` 作为唯一 ID。`provider_driver` 必须是全小写的规范机器 ID，建议限制为 `[a-z0-9][a-z0-9-]*`；文件名必须严格等于 `<provider_driver>.json`。`display_name` 是可选、区分大小写的人类可读名称，不参与查找、覆盖或文件命名；缺省或为空时仅在展示层回退为 `provider_driver`：

```text
driver_metadata/openrouter.json
                  └──────── provider_driver = "openrouter"
```

云 manifest、缓存目录、local override 和 system-config override 全部使用同一 ID，不再使用程序中的 provider 名称映射表。

### 6.2 内置文档自动注册

为 `aicc` 增加只使用 Rust 标准库的 `build.rs`：

1. 扫描 `driver_metadata/*.json`；
2. 按文件名排序；
3. 生成到 `OUT_DIR/builtin_driver_metadata.rs`；
4. 生成 `(provider_driver, include_str!(...))` 静态 catalog；
5. 输出 `cargo:rerun-if-changed=driver_metadata`。

构建或测试必须校验：

- 文件名和文档 ID 一致；
- `provider_driver` 满足全小写 ID 约束；`display_name` 若提供则为非空展示文本，缺省时展示值为 `provider_driver`；
- ID 不重复；
- schema、required features 和所有 rules 合法。

这样新增 metadata 文件不需要修改 `metadata_resolver.rs::load_builtin_driver_metadata` 的 `match`。

### 6.3 默认模型列表

Metadata 文档中的 `models` 同时是 exact rules 和默认模型列表。按 JSON 中的顺序读取所有带非空 `id` 且 `exclude != true` 的条目，得到该 `provider_driver` 的默认 `provider_model_id` 列表。`patterns`、`defaults`、`version_rules` 和 `variants` 只能解析或扩展已有模型 ID，不能凭空生成默认模型。

协议 adapter 不再维护 `DEFAULT_OPENAI_MODELS`、`DEFAULT_CLAUDE_MODELS`、`DEFAULT_GEMINI_*`、`DEFAULT_MINIMAX_MODELS` 或 fal 默认模型常量，也不从 provider 产品名选择默认列表。Provider instance 第一次构造、尚未完成远端 discovery 时，直接用选中 metadata 的默认列表构造初始 inventory。

远端 discovery 成功后，远端返回的模型 ID 列表替代默认候选列表，再由同一 metadata 解析；不能与默认列表做无条件 union，否则已经从 provider 下线的默认模型会继续被路由。后续 discovery 失败时保留最近一次成功 inventory；若从未成功，则继续使用 metadata 默认 inventory。云更新替换 metadata 后，新的 `models` 也同时成为所有引用实例的新默认列表，但不会改变实例 URL、凭据或协议。

### 6.4 模型名称映射

必须区分两个名称：

| 字段 | 含义 | 示例 |
| --- | --- | --- |
| `provider_model_id` | `/models` 返回、调用该实例 API 时使用的渠道模型名 | `openai/gpt-5.5` |
| `origin_model_id` | metadata 映射得到、用于查询模型定义的原始模型名 | `gpt-5.5` |

名称映射继续使用当前已经实现的 `DriverOriginIdentity`，不新增 `OriginalModelRef` 或新的标识字段：

```text
DriverOriginIdentity {
    driver: "openai",
    model: "gpt-5.5"
}
```

OpenRouter metadata 可以把 `openai/gpt-5.5` 映射为 `driver = "openai"`、`model = "gpt-5.5"`；OpenAI metadata 则使用现有 identity fallback。不能在 Rust 中通过 `/`、provider 名称或模型前缀猜测原始模型名。

映射由当前实例选择的 metadata 中的 exact mapping、pattern mapping、alias table 和明确的 identity/fallback policy 定义。映射失败时按 metadata policy 排除或使用保守 fallback，不能由 adapter 自行猜测。

resolver 始终使用 instance 的 `provider_driver` 所选择的当前 metadata 文档。该文档中的 `models[].id`、`patterns[].pattern`、`version_rules[].model_pattern`、`variants[].model_pattern` 以及 exclude/overlay 等模型匹配条件，都匹配 provider 实际返回并最终用于调用的 `provider_model_id`，不能改用 `origin_model_id` 或 `DriverOriginIdentity`。因此 OpenRouter 可直接用 `openai/gpt-5.5`、`:free`、`latest` 等渠道名称定义自身的准入、价格和 variant 规则，不需要先把它们改写为原厂名称。

名称映射是同一 resolver 中与规则匹配并行的另一项工作：它生成现有 `model_driver` 和 `origin_model_id`，并可供 mount 模板展开，但不会改变规则的匹配输入，也不会切换 metadata 文档。OpenRouter 始终使用独立的 `openrouter.json`，OpenAI 始终使用独立的 `openai.json`，两者的更新和解析互不覆盖。即使二者映射到相同的原始模型，也分别按各自的渠道模型名命中各自文档中的规则。

最终 `ModelMetadata` 同时保留：

- `provider_model_id`：实际发给当前 provider instance；
- `origin_model_id`：来自 `origin.model`，用于模型语义解析；
- `model_driver`：来自 `origin.driver`，表示原始模型定义来源；
- `ProviderInventory.provider_driver`：当前 instance 选择的 metadata ID。

### 6.5 Inventory 解析流程

OpenRouter 复用 `OpenAIProvider` 的通用 inventory 构造流程，但不复用 OpenAI 的渠道模型 ID。它通过 instance 的 `provider_driver = "openrouter"` 选择独立的 `openrouter.json`，初始使用该文档 `models` 定义的默认 OpenRouter 模型名，discovery 成功后改用 `/models` 返回的模型名。`openrouter.json` 直接用这些 `provider_model_id` 匹配 exact、pattern、defaults、version rules 和 variants，不进入 `openai.json`；origin mapping 只负责补充原始模型来源。

远端发现的所有非空、去重渠道模型名都进入统一 resolver：

```text
API protocol 发现 provider_model_id
    -> trim + case-insensitive dedupe
    -> 当前 provider_driver 对应的 metadata 按 provider_model_id 匹配规则
    -> exact / pattern / defaults / exclude / version rules / variants
    -> 当前 metadata 的 name mapping 生成 DriverOriginIdentity(driver, model)
    -> capability / api_types / mounts / pricing / origin
    -> ProviderInventory（同时保留渠道名与原始名）
```

因此 `normalize_remote_model_ids` 只应保留与 provider 无关的 trim/dedupe，或者改成通用名称归一化函数。exact、pattern、defaults、exclude、version rules、variants 和 overlay 全部使用 `/models` 返回的 `provider_model_id`；name mapping 也以同一个渠道名称为输入，但输出只用于来源字段和模板展开。`is_text2image_model_name`、`is_supported_llm_model_name` 以及按 OpenRouter 补模型的分支都可以删除。改造完成后，adapter 不再识别 GPT、Claude、Gemini、image、embedding、ASR、TTS；名称匹配、来源映射和模型语义由 metadata 负责。OpenRouter whitelist 当前已经位于 metadata 中，但现有末尾 `pattern = "*" / exclude = true` 只放行 OpenAI 模型；实现本方案时必须在 catch-all 之前增加 Anthropic、Google 等渠道规则，不能继续把 OpenRouter 等同于 OpenAI 模型集合。

SN 不在本方案范围内。后续为 SN 单独实现定制 API 协议时，再独立设计其模型发现和库存构造，不在本次改造中增加 SN metadata 兼容字段或分支。

## 7. Provider Instance 构造

实例配置结构建议为：

```rust
struct ProviderInstanceSettings {
    provider_instance_name: String,
    api_protocol_id: String,
    provider_driver: String,
    base_url: String,
    auth: AuthSettings,
}
```

构造流程：

1. 按 `api_protocol_id` 从程序内置的 `XxxProvider` 对象列表选择实现；
2. 按 `provider_driver` 从 metadata catalog 取得当前激活文档；
3. 使用 instance 的 URL 和 auth 创建协议客户端；
4. 读取 metadata `models` 中未排除的 ID，构造默认 inventory；
5. 由协议实现异步发现模型 ID，成功后替换默认候选列表；
6. 由指定 metadata 按渠道模型名匹配规则、映射 `DriverOriginIdentity` 并构造 inventory；
7. 校验 inventory 中的 API types 不超出协议实现支持范围；
8. 以 `provider_instance_name` 注册实例。

若协议不存在、metadata 不存在或两者能力不兼容，实例构造必须失败并给出具体错误，不能猜测或回退到 OpenAI。

同一个 metadata 可以被多个实例复用；同一个协议也可以搭配多个 metadata。云更新 metadata 后，所有引用该 ID 的实例在原子刷新 inventory 时看到新版本，但实例名称、URL 和凭据保持不变。

## 8. 对当前硬编码的处理

| 当前代码 | 改造方向 |
| --- | --- |
| `metadata_resolver::load_builtin_driver_metadata` provider `match` | 自动生成 metadata catalog |
| `openai::default_inventory` 的 OpenRouter 分支 | 删除 provider 名称判断；通用流程只消费 instance 自己的渠道模型 ID，并按自身 `provider_driver` 加载独立 metadata |
| `openai::build_inventory_from_models` 的 OpenRouter 分支 | metadata resolver |
| `normalize_remote_model_ids` 的 provider 判断 | 仅做通用 trim/dedupe；metadata 规则和来源映射都读取渠道模型名 |
| `main.rs` 的 provider type 到协议/driver 映射 | instance 显式提交两个 ID |
| `default_endpoint(provider_type)` | 内置实例显式配置；自定义实例由 UI 输入 |
| `extra.provider = "openai"` | 记录 instance name、protocol ID 和 metadata ID |
| 根据 instance name/base URL 推断 driver | 删除 |

### 8.1 其他 Provider 典型 Case Review

判断标准：

- 模型名称、类型、能力、价格、版本、mount 和准入属于 metadata，本方案应消除对应硬编码。
- endpoint 路径、header、请求体、响应解析和 provider-native dialect 属于 `XxxProvider` API 协议实现，应保留。
- 内置实例的默认 URL 可以保留在 instance preset；默认模型列表统一来自所选 metadata 的 `models`，不能留在协议 adapter 或 instance 私有配置中。
- 远端接口返回的动态状态可以由协议实现读取，但不能通过本地模型名称字符串重新推导 metadata 已定义的语义。

#### OpenRouter 跨原厂模型

| 当前 Case | Review | 本方案结果 |
| --- | --- | --- |
| `OpenAIProvider` 只按 GPT、DALL-E 等名称识别模型 | OpenRouter 同一 API 会返回 Claude、Gemini 等模型，协议 adapter 不可能维护所有原厂名称 | **解决**：所有渠道模型名直接进入 `openrouter.json` resolver，API type 和 capability 不由 `openai.rs` 猜测 |
| `openrouter.json` 末尾用 `pattern = "*"` 排除所有未明确放行模型 | 当前 metadata 实际只覆盖 OpenAI 模型，无法体现 OpenRouter 的多原厂库存 | **需要扩展 metadata**：在 catch-all 之前按 `anthropic/claude-*`、`google/gemini-*` 等 provider 返回名定义规则；未描述模型继续保守排除 |
| `openai.rs` 按 origin model 名称执行价格 fallback | OpenRouter 渠道价格不等于原厂直连价格，且会把模型家族逻辑重新固化到协议代码 | **解决**：实际 cost、estimate cost 和路由估价只读取 resolved metadata pricing |
| 根据 `model_driver = claude/google-gemini` 调用对应 adapter 逻辑 | 模型来源不等于当前 instance 的 wire protocol | **禁止**：执行 adapter 只由 instance 的 `api_protocol_id` 决定；origin identity 仅用于来源标注和模板展开 |
| OpenRouter Claude/Gemini 的 feature、context、版本和 mount 依赖 `claude.rs` / `gemini.rs` | 同一模型通过不同协议访问时会出现语义缺失或分叉 | **解决**：这些声明式语义写入 `openrouter.json`，并按 OpenRouter 返回的 `provider_model_id` 匹配 |

#### Claude

| 当前 Case | Review | 本方案结果 |
| --- | --- | --- |
| `refresh_inventory_once` 只接受 `claude-*` | adapter 用模型名决定库存准入，属于 metadata 职责 | **解决**：协议返回的全部模型名进入现有 origin mapping 和 metadata rules |
| `price_per_1m_tokens` 按 `opus` / `haiku` 字符串估价 | 与 metadata pricing 重复，新增系列仍需改代码 | **解决，但需要迁移数据**：在 `claude.json` 填写 token pricing，运行时 cost 从已解析 `ModelMetadata.pricing` 获取 |
| provider 级 `capabilities` / `features` 默认值 | 把所有 Claude 模型视为同一能力，且无法被 OpenRouter 等其他协议复用 | **解决**：实例聚合能力从 resolved inventory 派生，逐模型语义由 metadata 定义 |
| `DEFAULT_CLAUDE_MODELS`、默认 URL | 模型列表属于 metadata，URL 属于实例 preset，二者都不应固化在协议 adapter | **拆分**：删除模型常量，由 `claude.json.models` 提供默认列表；默认 URL 放入内置 instance preset |
| `/messages`、`anthropic-version`、Claude content lowering | Anthropic API 协议本身 | **不解决，也不应删除** |

`effective_features_for_claude_model` 等名称 classifier 当前只在 `#[cfg(test)]` 下使用，不是生产调用链；实现方案时应删除或改成 metadata resolver 的测试，避免测试继续固化第二套模型规则。

#### Google Gemini

| 当前 Case | Review | 本方案结果 |
| --- | --- | --- |
| `classify_gemini_model` 按 `embedding`、`tts`、`lyria`、`veo`、`imagen` 等字符串分 bucket | 模型类型和 API type 属于 metadata | **解决**：所有 provider 返回名直接由 `gemini.json` 规则匹配，不再构造 adapter-side buckets |
| embedding/TTS/music/video bucket 为空时补 `DEFAULT_GEMINI_*` | adapter 维护第二份默认模型列表 | **解决**：删除默认常量和 bucket fallback；`gemini.json.models` 统一提供各 API type 的默认模型 |
| `prefer_alias_over_versioned`、`keep_only_max_gemini_version` | 按 Gemini 命名推断版本和 alias，属于 metadata version rules | **解决**：对 Gemini 返回的 `provider_model_id` 应用 metadata version rules |
| `price_per_1m_tokens` 和 image price 名称判断 | 与 metadata pricing 重复 | **解决，但需要迁移数据**：token、image 和其他计价写入 metadata，cost 从 resolved model 读取 |
| provider 级固定 capabilities/features | 把协议能力误当成所有 Gemini 模型的能力 | **解决**：协议只声明可执行 API type 上限，实例和模型能力从 resolved inventory 派生 |
| 根据远端 `supportedGenerationMethods`、弃用描述判断动态可用性 | 一部分是远端动态状态，不是静态模型定义 | **部分解决**：名称分类删除；远端明确声明的方法和 deprecation 状态可作为动态 health/availability 证据保留 |
| `GEMINI_*_ALLOWLIST`、`:generateContent`、`x-goog-api-key` | Gemini 请求协议 | **不解决，也不应删除** |

#### MiniMax

| 当前 Case | Review | 本方案结果 |
| --- | --- | --- |
| `price_per_1m_tokens` 按 `m1`、`coding`、`plan` 判断价格 | 模型价格属于 metadata | **解决，但需要迁移数据**：`minimax.json` 定义 token pricing |
| `extra.provider = "minimax"` | 把协议对象名称误当成 instance/metadata identity | **解决**：使用当前 instance 的 `provider_instance_name` 和 `provider_driver` |
| `ProtocolDialect::MiniMax` 对 tool result 的降级规则 | MiniMax 暴露的 Anthropic-compatible dialect 与 Claude 并不完全相同 | **不解决，也不应删除**：它是程序支持的 API 协议行为 |
| `DEFAULT_MINIMAX_MODELS` 和默认 endpoint | 模型列表与连接配置所有者不同 | **拆分**：默认模型迁入 `minimax.json.models`，默认 endpoint 保留在内置 instance preset |

#### fal

| 当前 Case | Review | 本方案结果 |
| --- | --- | --- |
| `FalProvider::new` 强制 `provider_driver = "fal"` | 阻止同一 fal API 协议搭配另一份 metadata | **解决**：使用 instance 已配置的 `provider_driver` |
| 按四组 settings model list 手工传入 API type、cost、latency | inventory 参数与 `fal.json` 重复 | **解决**：模型名统一交给 metadata resolver，API type、价格和延迟从 metadata 获取 |
| `run_method` 和 `estimate_cost` 再次硬编码每种 method 的价格 | 形成第三份价格真相源 | **解决，但需要改 cost 调用链**：按 resolved model pricing 计价 |
| image/audio/video method 到输入字段、URL 和 artifact parsing 的映射 | fal API 协议本身 | **不解决，也不应删除** |
| fal 内置的四个默认模型 | 与 metadata 模型定义重复 | **解决**：由 `fal.json.models` 提供默认列表，协议 adapter 不再保存模型常量 |

#### 管理面与公共代码

| 当前 Case | Review | 本方案结果 |
| --- | --- | --- |
| `load_builtin_driver_metadata` 手写 Claude/Gemini/fal/MiniMax 等名称和 alias | 新增 metadata 必须改 Rust | **解决**：构建期自动 catalog；instance 必须使用 metadata 的唯一 `provider_driver` |
| `section_for_provider_type`、`provider_driver_for_request` | 把产品名、协议和 metadata ID 绑定在一起 | **解决**：自定义 instance 分别提交 API 协议、`provider_driver` 和 URL |
| `default_endpoint(provider_type)` | 对自定义 provider 造成产品名依赖 | **解决**：自定义 URL 必填；内置 instance 的默认 endpoint 可以保留在其构造配置中 |
| provider validate 按 `openai/openrouter/anthropic/google/custom` 选择 discovery | 应按用户选择的 API 协议调用对应 `XxxProvider` 能力 | **解决名称耦合**：仍保留程序支持协议的必要分发 |
| `apply_provider_settings` 显式调用各 `register_xxx_providers` | 这是程序中已有 `XxxProvider` 对象列表 | **不解决，也不需要数据驱动化** |
| OpenAI/Claude/Gemini/MiniMax/fal 响应中的固定 `extra.provider` | 自定义 instance 会被错误标成协议实现名称 | **解决**：观测字段来自 instance，不使用字符串常量 |
| control panel 的 `AICC_PROVIDER_SECTIONS` 和 `collect_aicc_provider_instance_names` | 通过固定 settings section 扫描实例，新增自定义组合无法自动出现 | **解决**：管理面读取 AICC 标准 provider instance 列表，不再枚举 section 名称 |
| control panel 的 Claude/MiniMax/Gemini card 硬编码模型、能力和 endpoint | 运行数据与 inventory/instance 配置重复 | **部分解决**：模型和能力读取 inventory，URL 读取 instance；内置产品的展示名称、说明和默认 preset 可以保留 |
| node activation 的 OpenAI/Claude/Google/GLM token 表单 | 首次激活时支持的内置 provider preset，不是运行时 provider catalog | **不解决，也不要求自动数据驱动**；自定义 provider 在 control panel 中创建 |

#### 明确不在本方案范围

- SN 后续使用独立的定制 API 协议，本次不改造其 metadata、认证和计费逻辑。
- Provider API 的错误分类、重试、HTTP header、request lowering、response parsing 和 option allowlist 属于协议实现。
- 本方案不会消除所有字符串常量；只消除通过 provider/模型名字选择 metadata 语义或实例私有配置的分支。

## 9. 协议与文档影响

需要联动：

- `DriverMetadataDocument`：继续使用现有 `provider_driver` 唯一字段，增加面向 UI 的可选 `display_name` 和可选 `description`；展示名解析为 `display_name` 非空值，否则为 `provider_driver`；
- provider instance settings：`api_protocol_id`、`provider_driver`、`base_url` 必填；
- provider add/validate API：接收上述明确字段；
- 管理 UI：为“API 协议”和“模型适配方案”提供两个禁止自由输入的下拉框，选项分别来自程序支持的协议对象列表和已验证的 metadata catalog；
- metadata 云更新 manifest：继续以 `provider_driver` 为覆盖键；
- usage/trace：分别记录 instance name、protocol ID 和 metadata ID；
- `doc/aicc/driver_metadata_schema.md`、provider 添加指南和 API 文档同步更新。

这是 beta 2.2 breaking change，不保留 `provider_type -> provider_driver`、URL 推断或空字段 fallback。

## 10. 分阶段实现

### Phase 1：Metadata 与 inventory 去名称分支

1. 自动生成 metadata catalog；
2. 明确 `provider_driver` 只负责选择 metadata，不负责选择 API 协议；
3. 所有远端渠道模型名直接作为 metadata 规则匹配输入，并由同一 resolver 生成原始模型引用；
4. OpenRouter 复用 `OpenAIProvider` 通用 inventory 流程，但只使用自己的渠道模型 ID 和独立 `openrouter.json`；
5. 删除 OpenAI adapter 中的 OpenRouter inventory 分支；
6. 将 `openai.rs`、`claude.rs`、`gemini.rs`、`minimax.rs` 和 `fal.rs` 中的模型家族分类、能力、价格、版本、mount 和 variant 逻辑迁入各自 metadata；
7. 删除协议 adapter 中所有 `DEFAULT_*_MODELS` 和默认 bucket，统一从 metadata `models` 构造默认 inventory；
8. 协议 adapter 的 provider 级能力改为协议上限，实际 instance/model 能力从 resolved inventory 派生。

### Phase 2：协议与实例解耦

1. provider instance 显式保存 protocol ID 和 metadata ID；
2. 统一现有 `XxxProvider` 对象的协议选择入口；
3. 内置实例在程序中显式指定二者；
4. 自定义 provider API 接收两个明确 ID，UI 通过两个下拉框选择，不提供自由输入；
5. 删除 provider 名称、URL 和 protocol 的互相推断。

### Phase 3：管理面与观测统一

1. UI 使用“模型适配方案”；
2. 程序支持的协议对象列表和 metadata catalog 提供 UI 可选项；
3. trace/usage 分别记录三个 identity。

metadata 继承不是当前方案的前置条件。如后续确实需要共享规则，应单独定义继承、云更新层级、循环检测和 revision 语义。

## 11. 验收条件

1. 增加一个使用现有协议的新 metadata JSON 后，无需修改 Rust provider 分支即可创建实例；
2. 两个实例可以复用同一个 metadata，同时保留不同名称、URL 和凭据；
3. 同一 OpenAI API 协议可以分别搭配 OpenAI 和 OpenRouter metadata；
4. 云更新同一 metadata ID 后库存刷新，但实例名称、URL、auth 和 protocol ID 不变；
5. instance 无法提交 metadata patch；
6. 非法/重复/不存在的 metadata ID 明确失败；
7. 不存在的 protocol ID 明确失败；
8. metadata 声明了协议不支持的 API type 时明确失败；
9. OpenRouter 的规则按其 `provider_model_id` 匹配，名称映射、模型语义、准入、价格和 variants 都由独立 `openrouter.json` 生效；
10. metadata 的 `provider_driver` 与文件名均为全小写且严格一致；UI 优先显示保留大小写的 `display_name`，缺省或为空时显示 `provider_driver`；
11. 自定义 provider 的 API 协议和模型适配方案均通过下拉框选择，不能输入 catalog 之外的 ID；
12. OpenRouter 的 Claude/Gemini 模型只通过 `OpenAIProvider` 执行，能力和价格来自 `openrouter.json`，不调用 `claude.rs` / `gemini.rs` 的模型家族逻辑；
13. 每份 metadata 的 `models` 中未排除条目构成默认模型列表；discovery 成功时替换默认候选，失败时使用默认 inventory 或最近成功的 LKGS；
14. 协议 adapter 中不再保存默认模型常量，也不再通过 provider、模型品牌或模型家族名称决定 inventory、能力、价格、版本、mount 或 variant；
15. 代码中不再通过 provider 名称决定协议、endpoint 或模型解析；
16. `cargo test -p aicc`、workspace `cargo test` 与 `uv run buckyos-build.py` 通过。

## 12. 风险与边界

### 12.1 必须在同一批改造中处理

- **必须保持渠道名称匹配边界**：当前 `openrouter.json` 的 exact model、pattern、variants 和 version rules 都按 `openai/...` 等渠道名称匹配，现有 resolver 也以 `provider_model_id` 为输入。实现时必须为每类规则增加回归测试，防止误改为 origin 匹配而导致 inventory 变空、错误合并不同渠道变体或丢失价格。
- **机器 ID 与展示名不能混用**：`provider_driver` 和文件名必须保持全小写且严格一致；`display_name` 可以区分大小写并用于 UI，缺省时仅由展示层回退到 `provider_driver`。查找、云覆盖、缓存键和 instance 引用不得使用展示名或展示 fallback，也不能在运行时对不合法 ID 静默转小写，否则可能把两个配置合并为同一文档。
- **不能直接复用 OpenAI 的调用模型 ID**：`openai.json.models` 使用 `gpt-*`，`openrouter.json.models` 通常使用 `openai/gpt-*`。可以复用构造流程，不能跨 metadata 复用默认列表；远端 discovery 完成前应使用当前 metadata 自己的默认 inventory。
- **variants 必须始终使用渠道调用名**：variant eligibility、`provider_actual_model_id` 和最终 API 调用都以 `provider_model_id` 为基础；`origin_model_id` 只表示来源，不能把 `gpt-*` 发给需要 `openai/gpt-*` 的 endpoint。
- **ProviderState 必须按 instance 隔离**：当前 OpenAI-compatible history 只用 `provider_driver` 标记 opaque state。多个 instance 共享同一 metadata 时，一个 endpoint/account 的 response state 可能被另一个 instance 接受。应改用现有 `provider_instance_name` 作为 owner，或至少同时校验 instance；不能改用 `origin.driver`。
- **metadata refresh 当前不是全局原子操作**：`refresh_all_provider_inventories` 在每个异步任务完成后立即逐个写入 registry，某些 instance 失败时会出现同一 metadata generation 下新旧 inventory 混用。若要求一致切换，需要先构造并验证全部受影响 inventory，再一次性提交；失败则继续使用 LKGS。

### 12.2 需要明确接受或增加保护

- **独立 metadata 带来重复维护**：`openai.json` 与 `openrouter.json` 完全隔离，因此 OpenAI 新模型或价格更新不会自动传播到 OpenRouter。这避免相互干扰，但 metadata 维护者可能需要分别更新两份文档；名称映射不会自动继承另一份 JSON 的规则。
- **同一 metadata 的更新影响所有引用实例**：这是共享 catalog 的预期行为。instance 名称、URL 和 auth 不会改变，但其 inventory、路由、价格和 variants 会同时变化，不支持单 instance 私有 patch 或灰度版本。
- **活跃 metadata tombstone**：当前云协议允许 tombstone。删除仍被 instance 引用的 `provider_driver` 时，应拒绝 activation 或让这些 instance 保持 LKGS 并进入 degraded，不能静默落入 conservative fallback。
- **完整 `/models` 列表的资源开销**：删除 adapter 预过滤后，聚合 provider 的全部模型都会进入 mapping/resolver。需要对模型数量、ID 长度、解析时间设上限，并为 exact/pattern 建索引，避免 refresh 放大 CPU 和内存消耗。
- **价格双真相源迁移**：Claude、Gemini、MiniMax 和 fal 仍在 adapter 中计算实际 cost。迁移期间 metadata pricing 与 adapter 价格可能不一致；必须一次性切换 usage cost、estimate cost 和路由估价，不能长期保留 fallback 表。
- **跨协议模型的 metadata 覆盖完整性**：OpenRouter 新增 Claude、Gemini 或其他原厂模型时，origin mapping 成功不代表渠道语义已经完整。若 `openrouter.json` 没有匹配的 API type、capability、价格和参数约束，保守 fallback 可能导致模型不可路由或成本未知；发布检查必须覆盖所有已准入渠道模型，并对未知模型采用明确的保守 defaults/exclude。
- **默认列表可能过期**：`models` 同时承担 exact rule 和 discovery 前默认列表，因此保留一个已下线 exact rule 会让它重新进入初始 inventory。下线但仍需保留解析信息的条目必须标记 `exclude = true`，metadata 发布检查还应验证默认 ID 可调用；discovery 成功后不能继续 union 默认模型。
- **路由结果可能变化**：当前 `semantic_llm_family_mounts` 仍按 Qwen、DeepSeek、Kimi、GLM、Grok 名称硬编码 mount。若迁入 metadata，必须用 route snapshot 验证默认模型、版本选择、重复 mount 和候选权重没有意外变化。
- **远端动态信息不能丢失**：Gemini `supportedGenerationMethods`、deprecation 等属于 discovery 返回的动态事实。metadata 决定静态语义，但远端明确声明的不支持/弃用仍应合并为 health/availability，不能因为去硬编码而忽略。
- **协议与 metadata 组合校验**：两者正交但不是任意组合都有效，必须校验 metadata 暴露的 API types 不超过 `XxxProvider` 实现能力。
- **不能把 metadata 变成可执行协议 DSL**：过度配置化 HTTP header、请求转换或错误恢复会扩大远程 metadata 的执行边界和安全风险。metadata 只允许声明经过 schema 验证的模型语义；wire 行为变化仍通过受审查的协议代码实现。
- **管理面 breaking change**：provider add/validate、control panel 和 settings 必须同批修改；固定 section 扫描与新 instance 模型并存会造成实例遗漏或重复展示。
- **下拉选项依赖 catalog 可见性**：UI 不能临时输入尚未安装的协议或 metadata ID。catalog 加载失败、云更新尚未生效或管理 API 返回旧快照时，新模板不会出现在下拉框中；UI 应提示刷新或安装模板，不能偷偷降级为自由输入。

### 12.3 不应受到影响的边界

- 云 metadata 不能携带 endpoint、auth、protocol 或 billing 字段。
- `ProviderInventory.provider_driver` 继续表示当前 metadata/channel；`model_driver` 和 `origin_model_id` 只表示模型来源，不能用于 endpoint 选择、ProviderState owner 或 provider allow/deny。
- 特殊计费、HTTP header、request lowering、response parsing 和 option allowlist 仍属于产品策略或 API 协议实现。

## 13. 不采用的方案

- 增加 `is_openrouter()`、`is_aggregator()`：仍以 provider 名称驱动行为。
- 用一个集中式 provider descriptor 重新绑定协议、metadata 和 endpoint：只是把原来的耦合移动到一张表，新 provider 仍需改程序。
- 允许 instance 覆盖 model rules：会产生不可追踪的私有 metadata 分叉。
- 把 endpoint/auth/billing 放入云 metadata：破坏实例私有配置和安全边界。
- 为 OpenRouter 复制 OpenAI adapter：会产生两份相同协议实现。
