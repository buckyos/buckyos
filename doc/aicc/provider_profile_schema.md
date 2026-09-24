# AICC Provider Profile 与 Provider Rules Schema

状态：Beta 2.2 目标规范
范围：定义 Provider Profile、Protocol Adapter、Provider Rules、Model Driver 与 Pricing 的稳定边界。AICC 重构必须以本文为准；新 settings 不保留旧 `provider_driver` 的身份语义，已导出的公共 RPC/报告兼容字段不在此范围内删除。

## 1. Provider 分为两类

AICC 不应把所有 Provider 都实现成同一种声明式配置，但 Provider Profile ID 和 Model Driver ID 必须保持开放，不能成为客户端代码白名单。

### 1.1 内置专用 Provider

以下 Provider 应定制实现：

- OpenAI、Google Gemini、Anthropic 等原厂官方 Provider；
- OpenRouter 等影响力大、协议和模型规则具有明显特异性的聚合 Provider；
- 其他需要长期稳定支持、必须进入发布验收矩阵的 Provider。

专用 Provider 在程序中固定实现的是不可声明的执行逻辑：

- 认证算法和 API wire protocol codec；
- discovery 的网络交互、分页、限流和状态机；
- 同步、流式、异步任务及无法声明化的错误处理；
- Provider inventory 的通用合并算法；
- discovery 失败后的 LKGS 行为。

这些逻辑不作为外部 Provider 参数配置暴露。Provider API 升级且确实改变上述执行逻辑时，由专用实现和测试一起升级。

专用 Provider 仍然必须使用本厂商独立的 `.provider.json`，并复用 Model Driver metadata。模型身份映射、moving alias、Provider variant、operation 选择、请求参数差异、能力收窄和静态价格等常规内容必须优先写入 `.provider.json`，不能因为 Provider 是内置专用实现就硬编码。只有统一配置 schema 无法安全表达的个性逻辑才允许留在代码中。

### 1.2 配置型 Provider

复用客户端现有 Protocol Adapter、且不需要专用 discovery、签名认证、动态登录或任务状态机的渠道，应作为配置型 Provider 接入。配置型 Provider 可以是云 catalog 正式发布的独立 Provider Profile，也可以是用户自行添加的 `custom` Provider。典型情况是兼容现有协议的厂商、小型 Provider 或用户自建代理。

配置型 Provider 适合处理：

- OpenAI-compatible 等已有协议的兼容服务；
- 模型名增加固定前后缀的代理服务；
- 只支持少量 Model Driver 的小型聚合平台；
- 少量模型需要指定不同 operation；
- Provider 无法查询实时价格，需要配置渠道默认价格。

正式发布的配置型 Provider 由独立 Known Provider 和 Provider Rules 定义。运行时动态枚举有效 catalog 中的全部 Known Provider，为未注册专用行为的 Profile 按 `protocol_adapter_id` 的协议族装配标准模型发现；发现失败时才合并 Provider Rules `models[]` 与 `metadata_drivers` 明确引用的 Model Driver exact `models[]` 形成静态兜底 inventory。Provider Rules 的显式 `exclude` 优先，pattern 因不能枚举具体模型而只参与匹配。metadata 更新后该兜底 inventory 必须随新 snapshot 重建。Known Provider 引用的 `protocol_adapter_id` 必须已经存在于当前客户端的 runtime registry。

`custom` Provider 的默认 Provider Rules 是 `{}`。它不获得任何厂商专用命名映射或参数特判：discovery 返回的 `provider_model_id` 同时作为待解析的 `origin_model_id`，按原名依次匹配系统当前安装的全部 Model Driver catalog。必须唯一命中才能取得对应原厂、默认参数和默认行为；多重命中按歧义拒绝，完全未命中进入统一 conservative fallback。标准名 `gpt-5.6-sol` 可以直接匹配，`openai/gpt-5.6-sol` 之类渠道前缀不会被自动去除。

如果一个 Provider 需要复杂状态机、特殊认证算法、新的流式格式或特殊错误恢复，应在先完成声明化评审后升级为专用 Provider。成为专用 Provider 只增加必要的行为实现，不取消其 `.provider.json`；大量模型专用分支通常说明规则尚未正确配置化，不能作为直接写代码的理由。

## 2. 配置文件原则

Provider 参数配置是官方支持 Provider 的渠道声明层，不是可执行 Provider manifest。每个被系统官方收录的 Provider 厂商都必须有自己的 `.provider.json`。`{}` 专门表示未被官方收录的 `custom` Provider 使用标准协议和标准模型名，不表示官方 Provider 可以省略其配置。

官方 `.provider.json` 打包成 NDN Provider Rules catalog 时，发布工具必须根据文件归属和 manifest 补齐 `format`、`schema_version`、`schema_revision`、`revision_seq`、`provider_profile_id` 等 catalog envelope；AICC 运行时消费的是带完整 envelope 的发布对象。`custom` Provider 的 `{}` 是运行时生成的标准空规则体，不需要伪造一个已经被官方发布的厂商 catalog。

模型参数和 Provider 参数必须按厂商拆分并分目录保存：

- 同一原厂商的全部模型参数集中在一个独立文件中，文件名为 `<原厂商名（小写）>.model.json`，存放在 `models/` 目录，例如 `models/openai.model.json`、`models/anthropic.model.json`；
- Provider 厂商包括原厂和聚合中间商。每个 Provider 厂商的参数集中在一个独立文件中，文件名为 `<厂商名（小写）>.provider.json`，存放在 `providers/` 目录，例如 `providers/openai.provider.json`、`providers/openrouter.provider.json`；
- 厂商名使用稳定的小写 slug。同一厂商不得按模型、API 代际或 Provider Instance 拆成多个同类参数文件；`models/` 与 `providers/` 必须分目录，不能仅依赖 `.model.json` / `.provider.json` 后缀避免命名冲突；
- 以上名称是 AICC 加载后的规范配置文件名。NDN 发布制品可以按发布协议在对象路径或父目录中携带 revision，但不能改变文件内部的厂商归属，也不能把多个厂商合并为一个配置文件。

同一 catalog 身份可能同时存在于 `builtin`、`cloud`、`local`、`system-config` 四个来源。加载器按 `system-config > local > cloud > builtin` 选择最高优先级来源中的整个 JSON 文件，不在来源之间合并任何字段、map、数组、规则或默认值。某个高优先级来源没有该身份时，低优先级文件继续有效；例如 cloud 只提供 OpenAI 文件时，builtin MiniMax 不受影响。最终 catalog snapshot 是这些逐身份获胜文件的并集。

- 纯协议且不随 Provider 渠道变化的固定语义不写入配置；
- 所有字段均可省略；
- `custom` Provider 的空对象 `{}` 必须合法，表示使用已解析 Adapter 的标准协议行为，并按原始模型名搜索全部 Model Driver；
- 实际调用始终使用 Provider discovery 返回的原始 `provider_model_id`；
- 尽量复用当前 Driver metadata 已有字段和规则结构；
- schema 只表达数据差异，不试图声明式实现完整 Provider Adapter。
- Provider 的特殊 dialect 也必须首先尝试用 `.provider.json` 中有界、可校验的规则表达；现有 schema 不足时，优先评审并扩展统一 schema，不能直接把常规差异写入厂商代码。

以下内容由程序固定，不进入配置：

- protocol adapter 的具体实现；
- refresh interval；
- 无匹配时使用 conservative fallback；
- 多个 Model Driver 同时匹配时拒绝解析；
- discovery 失败策略；
- 配置加载、schema 解析，以及由 NDN `metadata_target_seq` 和 Provider `metadata_applied_seq` 驱动的全局库存收敛机制。

Provider Instance 的名称、凭据、区域和用户自定义 `base_url` 属于实例私有配置，也不进入可云更新的 Provider 参数文件。

### 2.1 基础协议与派生 Adapter

OpenAI、Claude、Google Gemini 必须各自拥有专门实现、独立注册和独立验收的协议族。协议族不是可执行 Adapter；同一厂商的新旧 API 形态使用不同的内部 `protocol_adapter_id`，不能在一个 Adapter 内按 endpoint 能力或 Provider ID 切换。基础协议首先实现并维护官方推荐的新接口；历史接口不要求预先完整实现，只有首个真实 Provider 需要某个历史 API 代际时才增加对应 Adapter。这个 Adapter 属于协议族并可被所有兼容 Provider 复用，不属于首个触发需求的派生 Provider，也不能在后续 Provider 中重复实现。

内置厂商 Adapter 可以复用基础协议，并声明语义上的子类关系。派生 Adapter 是声明式配置无法表达时的最后手段，不是承载厂商参数表的默认位置：

```text
derived protocol_adapter_id
  -> base_adapter_id
  -> override/extend auth, endpoint, discovery or selected operations
  -> delegate unchanged wire behavior to the base adapter
```

“子类”只约束语义和依赖方向，实现可以采用继承、组合、委托或共享无状态协议组件。必须满足：

- 派生 Adapter 使用独立 `protocol_adapter_id`，不能冒充基础 Adapter；
- 依赖从派生 Adapter 指向基础 Adapter，基础 Adapter 不引用派生 Adapter；
- 基础 Adapter 不读取派生 Provider 的配置字段，不按 Provider ID 分支；
- 派生 Adapter 只覆盖差异点，未覆盖行为保持基础协议语义；
- 删除派生 Adapter、Profile、Rules 和测试后，基础 Adapter 的代码、schema 和行为不变。

初始 registry 关系至少包括：

| `protocol_family_id` | `protocol_adapter_id` | `base_adapter_id` | `component_adapter_ids` | 定位 |
| --- | --- | --- | --- | --- |
| `openai` | `openai-responses` | 无 | 无 | OpenAI 官方默认的新接口实现 |
| `openai` | `openai-chat-completions` | 无 | 无 | 首个真实需求出现时才注册，之后由兼容 Provider 共享的 Chat Completions 实现 |
| `openai` | `openai-completions` | 无 | 无 | 首个真实需求出现时才注册，之后由兼容 Provider 共享的旧 Text Completions 实现 |
| `claude` | `claude-messages` | 无 | 无 | Claude 官方默认 Messages 实现 |
| `claude` | `claude-completions` | 无 | 无 | 按首次真实需求实现，之后在协议族内共享 |
| `gemini` | `gemini-interactions` | 无 | 无 | Gemini 官方默认的新接口实现 |
| `gemini` | `gemini-generate-content` | 无 | 无 | 按首次真实需求实现，之后在协议族内共享 |
| `openai` | `sn-openai` | `openai-responses` | 无 | SN 鉴权扩展，当前复用 Responses 实现 |
| `openai` | `openrouter-responses` | `openai-responses` | 无 | OpenRouter 渠道扩展，复用 OpenResponses 兼容接口 |
| `glm` | `glm-chat` | `openai-chat-completions` | `openai-chat-completions`, `glm-media` | GLM 多协议复合 Adapter |
| `minimax` | `minimax-messages` | `claude-messages` | `claude-messages`, `minimax-media` | MiniMax 多协议复合 Adapter |

新接口 Adapter 与兼容 Adapter 是平级实现。兼容 Adapter 不继承新接口 Adapter，也不通过调用新接口失败后回退旧接口。两者只允许复用低层、无状态且协议中立的组件，例如 HTTP transport、SSE framing、通用 JSON/错误工具和 AICC normalized IR；endpoint path、request schema、response event、错误映射和能力声明保持各自内聚。

同一个历史 API 代际只实现一份共享 Adapter。Provider 没有额外差异时，Provider Profile 或 Instance 直接保存这个 Adapter ID。确有渠道差异时，先用 `.provider.json` 的 operation、provider options、request rules、能力收窄及其它受限声明表达；只有不同 wire envelope、流事件状态机、签名/动态认证算法、任务生命周期或无法声明化的错误语义，才建立独立派生 Adapter，并用 `base_adapter_id` 指向共享历史 Adapter。多个派生 Adapter 可以引用同一个历史 Adapter，各自只实现剩余的最小逻辑差异，不复制历史 wire protocol，也不在代码中保存可由 Provider Rules 表达的参数表。

Provider Profile/Rules 必须在路由前得到一个确定的 Adapter 和 operation。内置 Provider 的默认选择由 Known Provider catalog 和 `.provider.json` 固定；用户添加 `custom` Provider 时只选择或识别 OpenAI、Claude、Gemini 等协议族，不选择 API 代际。接入测试只枚举无 `base_adapter_id` 且声明 `probe_path` 的协议族级 Adapter，按“官方新接口优先、运行时已注册的历史接口其次”的 `probe_priority` 顺序发送带认证的空 JSON 探测，成功识别后把 resolved `protocol_adapter_id` 固化到 Provider Instance。只有 `404/405/501` 继续下一候选；其它 4xx 证明接口存在，认证、限流、网络和 5xx 直接报告，不能被误判成历史接口需求。运行时只使用已固化 Adapter，不重新探测，也不在一次调用中静默切换新旧 Adapter。

一个 Provider 需要多套 protocol client 时，不把实例字段扩成无约束的 Adapter 数组。
每套独立 wire 协议先注册可单独测试和复用的 Adapter，再由 Provider 的复合 Adapter 通过
`component_adapter_ids` 显式声明组件，并按 operation 绑定对应 codec。当前 `glm-chat`
由 `openai-chat-completions + glm-media` 组成，`minimax-messages` 由
`claude-messages + minimax-media` 组成；GLM/MiniMax 复合 Adapter 使用自己的
`protocol_family_id`，不再冒充 OpenAI/Claude。Registry 要求组件先注册且引用不可悬空，
但这两个 family 只供内置 Known Provider 使用。Provider Rules JSON 通过
`custom_provider_adapters` 精确列出允许用户自定义 Provider 复用的 Adapter；该数组默认为
空，可以随 catalog 云更新。`protocol_adapter.list` 根据当前 catalog snapshot 为每个
Adapter 计算 `custom_provider_selectable`，表单只汇总该值为 `true` 的 Adapter 所属
family，后端也必须拒绝显式提交未列入数组的 Adapter。当前只有
`openai.provider.json`、`claude.provider.json`、`gemini.provider.json` 开放各自通用基础
Adapter；GLM、MiniMax、FAL 及具体渠道派生 Adapter 均不进入选择列表。

GLM/MiniMax 的 LLM wire 差异仍优先通过各自 `.provider.json` 的 request rules、
operation 映射和 converter 表达；独立 family 的主要理由是它们还组合了无法由通用
规则安全表达的图片、语音、音乐和异步视频协议，而不是文本接口本身与基础协议完全不同。
`protocol_adapter.list` 会返回组件关系。一次路由仍在调用前得到唯一的
`composite adapter + operation + codec`，不会运行时试探或静默切换。

### 2.1.1 可执行绑定表的事实源

完整的 `provider × api_type × operation × codec` 关系见
[`provider_operation_bindings.md`](provider_operation_bindings.md)。
运行时通过 `protocol_adapter.list` 返回当前注册的 Adapter、operation、API type 和执行模式；
Provider Rules 再给出每个模型的 operation 选择。`call::tests::every_builtin_provider_operation_has_a_golden_lowering_binding`
从这两份事实源生成、去重并校验全部内置绑定及文档（当前 71 条）。因此新增或删除绑定必须修改
metadata/Adapter，并同步更新由测试强制校验的绑定文档。

### 2.2 SN Provider 的 OpenAI 子类语义

SN Provider 当前使用独立的 `sn-openai` Protocol Adapter，属于 `openai` 协议族，并声明 `base_adapter_id: "openai-responses"`。它复用 OpenAI Responses 请求、响应、stream、错误和 operation 语义，SN 特性只实现在派生层。

SN Provider 支持两种显式且互斥的认证模式：

```json
{
  "auth": {
    "mode": "api_key",
    "credential_ref": "system-config://secrets/aicc/sn-main"
  }
}
```

```json
{
  "auth": {
    "mode": "dynamic_login",
    "login_profile": "device_jwt",
    "login_endpoint": "https://sn.example/api/user/login_by_device_token"
  }
}
```

- `api_key` 模式与 OpenAI Bearer API Key 方式一致。
- `dynamic_login` 模式由 SN 派生层在运行时登录、缓存并按过期时间刷新 token，再把已解析的 Bearer credential 交给 OpenAI 基础调用路径。
- 动态 token 不进入 Provider catalog、inventory、trace、日志或持久 metadata；并发刷新需要合并，认证失败只按 SN 认证错误返回。
- OpenAI 基础 Adapter 只消费已解析的认证材料，不知道 token 来自静态 API Key 还是 SN 登录。
- 不允许把动态登录作为 OpenAI Adapter 的可选分支；这保证 SN 将来采用独立协议时可以干净拆除。

GLM JWT 使用同一个 `api_key` 模式并显式选择 typed credential variant：

```json
{
  "auth": {
    "mode": "api_key",
    "credential_ref": "system-config://secrets/aicc/glm-main",
    "credential_kind": "glm_jwt"
  }
}
```

其他内置厂商也可以使用同样的派生 Adapter 模式，但必须有独立 ID、明确差异面和基础/派生两层验收。

## 3. Model Driver 与 Provider 配置边界

### 3.0 Known Provider typed configuration

Known Provider catalog schema v1 是 Provider Profile 默认静态配置的唯一 metadata 来源。每项必须直接包含 typed `credential`、`connection` 与 `discovery_behavior_id`，可按需声明 `dynamic_login_behavior_id`、`connection_behavior_id`，不得从 `ui_hints` 推断。`CatalogSnapshot::resolve_provider_configuration()` 同时解析 Known Provider 和其 `provider_rules_id`，校验 Rules 存在且 identity 一致后，返回默认配置与稳定 behavior IDs。

行为 registry 以稳定 behavior ID 注册 discovery、动态登录或其它不可声明执行行为，不是 Provider Profile 白名单。Known Provider 必须在 metadata 中选择已注册 discovery behavior；通用 OpenAI-compatible/Claude/Gemini discovery 也有稳定 ID，可由任意新 Profile 复用。refresh 时从当前 Provider Rules 和明确引用的 Model Driver exact models 重建仅在机器发现失败时启用的 default inventory。任何未知或冲突 behavior 均拒绝装配，不允许读取 `ui_hints`、按 Provider ID 猜测或静默 first-match。

可选 credential 由 typed `credential_variants[]` 声明，实例在 `auth.mode=api_key` 时用 `credential_kind` 显式选择；省略则使用 `credential` 默认值。区域入口由 typed `connection.region_base_urls` 声明，只有实例未显式提供 `base_url` 时才按解析后的 region 选择。GLM 的 `glm_jwt` 和 GLM/MiniMax 的区域入口均通过这两个 typed 字段进入 production registry。SN 的 `device_jwt` 是 SN 登录实现支持的稳定行为 ID，由显式 `auth.login_profile` 选择和校验，不从可选的 `ui_hints` 推断。

### 3.1 Model Driver metadata 管理

| 字段 | 说明 |
| --- | --- |
| `models` / `patterns` | 对原厂模型名的 exact/pattern 匹配规则 |
| `parameter_scale` | 模型参数规模或分类 |
| `api_types` | 模型固有的 AICC 能力类型 |
| `logical_mounts` | 模型家族和逻辑目录挂载 |
| `capabilities` | 模型固有能力和上下文限制 |
| `quality_score` | 与交付渠道无关的模型质量估计 |
| `version_rules` | 家族、tier、版本排序和稳定性规则 |
| `variants` | 对模型身份、路由和审计有意义的语义 variant |
| 默认价格 | 已无此字段：价格统一在顶层 `model_pricing` 表中声明 |

Model Driver 的 variant 定义语义身份，例如 `reasoning-high`，并可携带原厂默认 `provider_options`。配置型 Provider 命中该具体模型的任一 variant 时，由 Provider 配置中的 `variants` 完整定义该模型的 variant 集合和请求参数；完全未命中时才使用 Model Driver 默认值。

variant 名称是 Model Driver 与 Provider Rules 共用的封闭词汇表，Model Driver 的 `variants[].name` 和 Provider 的 `variants[].variant` 都必须使用 `driver_metadata_schema.md` 中 Variant naming 定义的统一档位名（`reasoning-none` / `reasoning-mini` / `reasoning-low` / `reasoning-medium` / `reasoning-high` / `reasoning-xhigh` / `reasoning-max`）。厂商自己的档位名（`effort-*`、`thinking-*`、`minimal`、`normal` 等）只能在 `provider_options` 中表达，不得作为 variant 名。

### 3.2 配置型 Provider 管理

| 用途 | 配置字段 |
| --- | --- |
| 允许自定义 Provider 复用的协议 Adapter | `custom_provider_adapters` |
| 限定参与匹配的 Model Driver metadata | `metadata_drivers` |
| Provider 厂商 slug 映射 | `origin_provider_aliases` |
| `provider_model_id` 到原厂身份的确定性映射 | `origin_mappings` |
| 渠道专属排除规则 | `models[].exclude` / `patterns[].exclude` |
| 选择按渠道模型名、原厂模型名或其它维度匹配 | `match: MatchRule`；字符串默认匹配渠道模型名 |
| Provider 请求参数 | `provider_options` / `variants` |
| 模型级请求默认值、改写和参数删除 | `request_rules` |
| Provider 渠道价格 | `model_pricing`（与技术规则并列的独立价格表） |
| 按质量、尺寸、时长等请求维度计价 | `model_pricing[].pricing.rules` |
| 按输入长度等用量档位计价 | `model_pricing[].pricing.tiers` |
| 按峰谷时段分时计价 | `model_pricing[].pricing.time_windows` |
| 模型使用的具体接口 | `operations` |
| Provider 无法提供的模型能力 | `remove_api_types` / `remove_features` |
| 渠道延迟提示 | `estimated_latency_ms`；只有毫秒数值进入调度 |

Provider 配置只能收窄 Model Driver 声明的能力，不能增加模型固有能力。

### 3.3 fal 的临时模型归属

当前 `fal-ai/esrgan`、`fal-ai/imageutils/rembg`、`fal-ai/deepfilternet3` 和 `fal-ai/video-upscaler` 尚无对应原厂 Provider 接入，暂时统一归属 `fal` Model Driver。其它配置随 fal 官方事实调整。

## 4. Custom Provider 的最小规则

```json
{}
```

`{}` 只承诺标准协议和标准模型名，不做 prefix/suffix stripping、vendor alias、moving alias 或其它重命名。系统按原始 `provider_model_id` 在全部 Model Driver 中执行 exact → pattern 匹配；唯一命中某个 Driver 后再合并该 Driver 的 defaults，零命中走 conservative fallback，多重命中拒绝。各 Driver 的 defaults 不能单独用于跨 Driver 猜测原厂。

conservative fallback 保留 discovery 声明的 API types；discovery 未声明时仅回退为 `llm`，并使用保留身份 `unclassified`。该模型可以按 exact model 调用，但没有 Model Driver logical mounts、variants 或静态能力，不会伪装成任一厂商模型。

当一个 Provider 需要以下可选字段时，它已经拥有厂商规则，不再属于纯 `{}` 语义；应创建或更新官方 `.provider.json`：

```json
{
  "custom_provider_adapters": [],
  "metadata_drivers": [],
  "static_inventory_models": [],
  "origin_provider_aliases": {},
  "origin_mappings": [],
  "models": [],
  "patterns": [],
  "model_pricing": [],
  "variants": []
}
```

- `custom_provider_adapters`：允许用户自定义 Provider 复用的已注册 Adapter ID；省略或空数组表示不开放。每项必须属于该 Provider 默认 Adapter 的 family，且不能被其它 Provider Rules 重复开放。
- `metadata_drivers`：参与匹配的 Model Driver 列表；省略时搜索系统当前安装的全部 Model Driver。
- `static_inventory_models`：无视模型发现结果、始终并入 inventory 的 `provider_model_id` 列表；每项必须同时存在于 `models[]`，由 catalog 引用校验保证规则和静态清单一致。`models[]` 本身只是全量精确规则，不会自动产生 inventory 模型，其中仅由 discovery 启用的模型不要放入 `static_inventory_models`。
- `origin_provider_aliases`：Provider 命名中的厂商 slug 到 Model Driver 名称的映射。
- `origin_mappings`：可以从命名确定性解析原厂身份时使用的特殊映射。
- `models`：按完整 `provider_model_id` 精确匹配的 Provider 规则。
- `patterns`：有序 Provider 规则；每项的 `match` 通常直接写匹配完整 `provider_model_id` 的 wildcard 字符串，多维条件才写对象。
- `variants`：将 Model Driver 语义 variant 转换为 Provider 请求参数；variant 名必须使用统一档位名，厂商档位差异写在 `provider_options`。

不增加 `refresh_interval_sec`、`on_no_match`、`on_ambiguous`、`failure_policy`、`protocol_adapter` 等程序固定字段。

`metadata_drivers` 显式为空数组表示不使用 Model Driver metadata，所有模型进入 conservative fallback；字段省略则搜索系统当前安装的全部 Model Driver。未配置 `origin_mappings` 时只能按原始完整模型名匹配，不得自动删除厂商前缀、后缀或别名。

## 5. 模型规则

`models` 和 `patterns` 使用 [match_rule.md](match_rule.md) 定义的统一 `MatchRule`。简单规则只写字符串 wildcard；只有同时约束多个维度时才使用对象。exact `models` 优先；未命中 exact 时，`patterns` 按数组顺序使用第一条匹配规则。

专用 Provider 与配置型 Provider 都复用现有模型规则和 resolver，也都从各自独立的 `.provider.json` 加载 Provider model rules。两者的区别只在于专用 Provider 可以注册配置无法表达的执行逻辑，而不是拥有代码内的第二份规则真相源。调用前可以产生临时的 resolved provider call，但它不是新的配置或真相源。

完整的可选配置项如下：

| 配置项 | 默认值 | 用途 | 来源 |
| --- | --- | --- | --- |
| `id` | 无 | `models` 中精确匹配模型；内部归一化为单维 `MatchRule` | 复用现有字段 |
| `match` | 无 | `patterns` 中的 `MatchRule`；通常直接写 wildcard 字符串 | 统一字段 |
| `exclude` | `false` | 从当前 Provider inventory 排除模型 | 从 Model Driver metadata 移入 |
| `operations` | `{}` | method/api_type 到 adapter operation 的映射 | 新增 |
| `provider_options` | `{}` | 调用该模型时附加的 Provider 参数 | 从 Model Driver metadata 移入 |
| `canonical_fields` | `{}` | 以 Rust converter 和失败策略覆盖 Model Driver 的 canonical 字段映射 | 新增 |
| `request_rules` | `[]` | 请求默认值、条件改写和不兼容参数删除 | 新增 |
| `pricing` | 无 | 不再是模型规则字段：价格声明在顶层 `model_pricing` 表中 | 已从 `models` / `patterns` 移出 |
| `remove_api_types` | `[]` | 删除当前 Provider 无法提供的 API type | 新增 |
| `remove_features` | `[]` | 删除当前 Provider 无法提供的 feature | 新增 |
| `estimated_latency_ms` | 无 | 渠道默认延迟估计 | 从 Model Driver metadata 移入 |

`model_pricing` 是与 `models` / `patterns` 并列的顶层数组，不属于单个模型规则。把价格从模型
规则里拆出来，是为了让"哪些模型走哪个接口、带哪些默认参数"继续由 `patterns` 通配批量声明，
而"每个模型的单价"逐个精确声明，两者互不牵连：此前为给出差异化价格而新增的精确条目，会同时
顶掉 `patterns` 的批量技术参数（exact 优先），拆开后不再有这个问题。

未配置的字段不覆盖 adapter 默认值。示例：

```json
{
  "match": "vendor/veo-3.1-*",
  "operations": {
    "video.txt2video": "videos.create"
  },
  "request_rules": [
    {
      "defaults": {
        "quality": "standard"
      }
    }
  ],
  "pricing": {
    "currency": "USD",
    "estimated_cost": 0.4,
    "unit": "request"
  },
  "remove_api_types": [],
  "remove_features": [],
  "provider_options": {}
}
```

### 5.1 匹配对象

Provider model rule 的字符串 `match` 默认匹配 `provider_model_id`。需要改用原厂模型身份或联合其它维度时才展开为对象，例如：

```json
{
  "match": {
    "origin_model_id": "gpt-5-*",
    "api_type": "llm"
  }
}
```

允许的模型身份维度包括：

- `provider_model_id`：默认值，用于渠道命名、排除和 operation 规则；
- `origin_model_id`：用于模型被 Provider 重命名后仍需应用的模型级 wire 参数和价格规则。

使用 `origin_model_id` 时，Model Driver 必须已经唯一匹配成功。配置规则不能修改实际调用使用的 `provider_model_id`。

### 5.2 Operation

`operations` 的 key 可以是 AICC method 或 api_type，解析优先级固定为：

```text
method exact key > api_type key > adapter default operation
```

例如同一 video api_type 下分别选择接口：

```json
{
  "operations": {
    "video.txt2video": "videos.create",
    "video.img2video": "videos.create",
    "video.video2video": "interactions.create"
  }
}
```

`model_driver` 不属于 Provider 模型规则。它是 Model Driver 唯一匹配后的解析结果；Provider 只通过 `metadata_drivers` 限定候选范围，少数确定性命名通过 `origin_mappings` 提供快捷映射。当前 metadata 规则中已有的 `model_driver` override 需要在拆分时重新审查，不复制到 Provider 配置。

operation 是现有 adapter 已实现的符号名称，不是任意 URL。adapter 自己知道 operation 使用的 endpoint、请求结构和异步流程。

### 5.3 Canonical field mapping

`canonical_fields` 的 key 是 typed request 中的非空 JSON Pointer，value 是映射策略。
`converter` 是 AICC 代码中固定实现的转换函数名；`fallback` 统一控制 canonical 字段
不存在，或字段存在但 converter 无法转换时的行为：

```json
{
  "canonical_fields": {
    "/voice": {
      "converter": "gemini_tts_voice_v1",
      "fallback": { "action": "default", "value": {} }
    }
  }
}
```

`fallback.action` 可为 `reject`、`omit` 或 `default`；`default`
必须提供 canonical `value`，catalog 加载时会用同一个 converter 验证该值。converter
是纯 Rust 函数，只负责值域转换及 exact/fuzzy 质量，不内置缺失、默认或失败策略。
metadata 不携带映射表、脚本或任意表达式。函数名是受 serde 校验的协议枚举，未知名称、
无法转换的默认值都会使 catalog 加载失败。`strict=true` 的显式要求失败时总是
reject，不执行 fallback；是否允许 fuzzy 由同一个 requirement 的 `allow_fuzzy`
决定。`strict` 默认值为 `false`。Typed TTS 当前自动构造非严格 requirement；
`VoiceSpec` 本身不携带匹配策略。

converter 按厂商对字段语义的演进命名，不包含首次采用该规则的模型名；模型规则只负责选择
对应版本。例如 OpenAI TTS voice v1 表示预置 voice ID，v2 在此基础上增加 instructions
语义。当前 converter 名称为：`passthrough`、`prompt`、`openai_tts_voice_v1`、
`openai_tts_voice_v2`、`gemini_tts_voice_v1`、
`minimax_tts_voice_v1`。Provider 对同一 pointer 的整个策略对象完整覆盖 Model Driver
默认策略。转换及策略结果的匹配质量仍按 exact → fuzzy → default → prompt 排序；严格
要求不能落到 default、omit 或 prompt。完成 canonical 映射后才执行 `request_rules`。

### 5.4 Request rules

`request_rules` 是有序列表。每条规则只有四个字段：

- `when`：可选条件；省略表示无条件执行；
- `defaults`：只填充尚未出现的字段；
- `set`：覆盖已有字段；
- `remove`：删除不兼容字段，使用 JSON Pointer。

`when` 使用统一 `MatchRule` 的多维对象形式，维度名是 normalized option 的 JSON Pointer；多个字段固定为 AND，数组值为 OR。简单等值条件直接写 `{ "/quality": "high" }`，不再使用 `path/op/value` 谓词对象，也不支持脚本、任意表达式或自定义函数。

以下规则可以替代 GPT nano 默认参数和 GPT/Codex sampling 参数特判：

```json
{
  "match": {
    "origin_model_id": "gpt-5-nano*"
  },
  "request_rules": [
    {
      "defaults": {
        "reasoning": {
          "effort": "minimal"
        },
        "text": {
          "verbosity": "low"
        }
      }
    },
    {
      "when": {
        "/reasoning/effort": {
          "not": "none"
        }
      },
      "remove": [
        "/temperature",
        "/top_p",
        "/logprobs",
        "/top_logprobs"
      ]
    }
  ]
}
```

条件基于 AICC 已归一化、准备交给 adapter 的 options，而不是直接查询任意原始 JSON。规则执行顺序固定为：Provider defaults、用户显式参数、条件 `set/remove`；因此用户参数通常覆盖默认值，但不能恢复 Provider 明确禁止的字段。

### 5.5 Pricing

价格声明在与 `models` / `patterns` 并列的顶层 `model_pricing` 表中，每项只允许 `id` 与 `match`
二选一：

```json
{
  "model_pricing": [
    { "id": "vendor/image-model", "pricing": { "currency": "USD", "unit": "image", "amount": 0.042 } },
    { "match": "vendor/fast-*", "pricing": { "currency": "USD", "input_token": 5e-07, "output_token": 2e-06 } }
  ]
}
```

解析顺序固定为：先查精确 `id`，未命中再按声明顺序取第一条命中的 `match`。Model Driver 侧以
`origin_model_id` 查找，Provider Rules 侧以 `provider_model_id` 查找。查找发生在技术规则解析
之后，与模型本身命中 `models` 还是 `patterns` 无关，因此通配技术规则与逐模型价格可以并存。

`pricing` 本身支持三类正交的计量方式：

- token 计量：`currency` 与 `input_token` / `output_token` / `cache_input_token`，都是**每 token**
  单价（官方"元/百万 Tokens"要除以 1e6）；
- 非 token 计量：`unit` + `amount`，`unit` 取值 `request`、`image`、`audio_second`、
  `video_second`、`character`、`second`（算力秒）、`megapixel`（百万像素）；同一份 `pricing` 内
  token 字段与 `unit` 互斥；
- `estimated_cost`：无法精确计算时的默认估值，只作展示与路由参考。

在计量之上还有三种取价修饰：

- `tiers`：按本次请求的**实际用量**选档，因此只能在结算时确定。`dimension` 取值
  `input_tokens`、`output_tokens`、`total_tokens`、`context_tokens`、`request_units`、
  `characters`；`mode` 为 `volume`（整单按命中档计价）或 `graduated`（逐档累进）；`steps[].up_to`
  是**不含**的上界，最后一档省略。适合厂商按输入长度分档的价表。
- `time_windows`：按**挂钟时间**分时取价，在请求时刻钉住。外层字段是默认（闲时）价，命中窗口
  只覆盖窗口内声明过的字段，其余继承。`from` / `to` 为本地 `HH:MM`，`from > to` 表示跨午夜；
  `days` 可限定星期；`utc_offset_minutes` 定义该分时表使用的时钟（默认 0 即 UTC，不感知夏令时，
  同一份 `pricing` 内必须一致）。窗口之间不得重叠。
- `rules`：按**请求参数**选价，在请求前确定，使用与 `request_rules.when` 相同的 `MatchRule`。
  `rules` 使用第一条命中的价格，均未命中时退回外层 `amount` / `estimated_cost`。`rules` 是
  Provider Rules 专属能力，Model Driver 的 `model_pricing` 声明它会被校验拒绝；`tiers` 与
  `time_windows` 两侧都可以使用。

例如 GPT Image 按 quality/size 计价：

```json
{
  "model_pricing": [
    {
      "id": "gpt-image-2",
      "pricing": {
        "currency": "USD",
        "unit": "image",
        "amount": 0.042,
        "rules": [
          {
            "when": {
              "/quality": "high",
              "/size": [
                "1536x1024",
                "1024x1536"
              ]
            },
            "amount": 0.167
          },
          {
            "when": {
              "/quality": "low"
            },
            "amount": 0.011
          }
        ]
      }
    }
  ]
}
```

结算时 image 单价乘以归一化请求中的生成数量，audio/video second 与 character 单价乘以归一化
时长或字符数。`second` / `megapixel` 目前没有 adapter 上报对应计数器，`completion_cost` 返回
unknown 而不是 0——先让价格口径可表达，等计数器落地即自动生效。按次计费的 `tiers` 不参与选档：
档位只在 token 计量下生效。

### 5.6 能力收窄

`remove_api_types` / `remove_features` 只能从 Model Driver 结果中删除能力。最终可执行能力固定取交集：

```text
Model Driver 静态能力
∩ Provider Adapter 已实现能力
∩ Provider 配置和 discovery 的可用能力
```

## 6. 匹配流程

```text
Provider discovery 获得 provider_model_id
    ↓
应用 Provider models / patterns 排除规则
    ↓
在 metadata_drivers 限定范围内搜索 Model Driver metadata
    ↓
唯一匹配一个 Model Driver
    ↓
确定 origin driver / origin model
    ↓
应用 Model Driver 的模型语义
    ↓
合并 operation、价格、请求参数和能力限制
    ↓
生成 Provider Instance 级 inventory
```

无匹配、冲突和 fallback 行为由程序统一处理，不由每份配置选择。

## 7. 价格优先级

价格优先级由程序固定：

```text
Provider 实时 discovery 价格
> Provider Rules 的 model_pricing（渠道价）
> Model Driver 的 model_pricing（原厂默认价）
```

两张 `model_pricing` 表里的静态价格都不能覆盖更新鲜的实时价格；渠道价可以覆盖原厂默认价。

## 8. OpenAI 官方 Provider 示例

OpenAI 是官方内置专用 Provider。程序固定实现 discovery 和调用协议，同时必须提供独立 `openai.provider.json`，明确限定原厂 metadata 和 operation，例如：

```json
{
  "metadata_drivers": ["openai"],
  "patterns": [
    {
      "match": "*",
      "operations": {
        "llm": "responses.create"
      }
    }
  ]
}
```

示例只展示边界，不取代完整 operation 表。OpenAI 官方 Provider 不使用 `{}` 的 custom 语义，也不依赖 Rust 中隐藏的 `metadata_drivers` 或 operation 映射。

## 9. OpenRouter 示例

OpenRouter 是内置专用 Provider，而不是配置型 Provider。

以下渠道规则应由 `openrouter.provider.json` 声明并纳入发布测试：

- 解析 `vendor/model` 命名并映射到候选 Model Driver；
- 维护 OpenRouter vendor slug 与 Model Driver 的别名关系；
- 排除 moving alias、Provider variant alias 和 OpenRouter 虚拟模型；
- 保留原始 `provider_model_id` 完成实际调用；
- 按模型和 AICC `api_type` 选择 OpenRouter Responses、embedding、rerank 等 operation；
- 从 OpenRouter discovery 获取价格并覆盖 `model_pricing` 里的静态价；
- 对可声明差异随 metadata catalog 进行版本发布。

OpenRouter 仍从 OpenAI、Claude、Gemini 等 Model Driver metadata 获取模型固有能力，候选范围、命名解析、排除规则、operation 和静态价格规则均以 `openrouter.provider.json` 为真相源。只有 Models API 交互、无法声明化的响应/事件解析等执行逻辑留在专用实现中。

## 10. Custom Provider 与正式渠道映射示例

假设用户自建 `example-proxy`，明确知道它兼容 OpenAI 协议，并且它原样提供 `gpt-5.6-sol`、`claude-sonnet-4-6` 等标准模型名。用户创建 `custom` Provider、选择 `openai` 协议族、填写连接和凭据；接入测试解析具体 Adapter 后，其 Provider Rules 为：

```json
{}
```

系统不会因为选择了 OpenAI 协议族就只搜索 OpenAI Model Driver，而是按原始模型名搜索全部 Model Driver。协议族决定怎么调用，模型名匹配决定模型来自哪个原厂以及采用什么默认 metadata。

如果该代理返回 `openai/gpt-5.6-sol`，空规则不会自动删除 `openai/`，因此不能把它当作 `gpt-5.6-sol`。若要让这种非标准命名成为系统正式支持的渠道行为，项目方必须为它发布独立 `.provider.json`，像 OpenRouter 一样显式声明 origin mapping，例如：

```json
{
  "metadata_drivers": [
    "openai",
    "claude"
  ],
  "origin_provider_aliases": {
    "anthropic": "claude"
  },
  "origin_mappings": [
    {
      "extract": {
        "source": "provider_model_id",
        "regex": "^(?<driver>[^/]+)/(?<model>.+)$"
      },
      "transforms": {
        "driver": [
          {
            "op": "lowercase"
          },
          {
            "op": "alias",
            "table": "origin_provider_aliases",
            "on_missing": "keep"
          }
        ],
        "model": [
          {
            "op": "trim"
          }
        ]
      }
    }
  ],
  "patterns": [
    {
      "match": "*:*",
      "exclude": true
    },
    {
      "match": "*/*latest*",
      "exclude": true
    },
    {
      "match": "openai/gpt-5*",
      "operations": {
        "llm": "chat.completions.create"
      }
    }
  ]
}
```

这类映射一旦存在，便是该 Provider 的官方渠道规则，不再属于 `{}` custom Provider 的默认行为。

## 11. 文件选择与规则解析语义

来源选择先于规则解析。同一 `provider_profile_id` 在多个来源出现时，只读取 `system-config > local > cloud > builtin` 中最高优先级的完整文件；下层同名文件不参与解析，也不提供缺失字段的默认值。下面的覆盖规则只用于“已选文件内部的规则”和程序定义的 schema/Adapter 默认值，不是跨来源 merge：

- map 按 key 覆盖；
- `models` 按 `id` 覆盖同名 exact rule；
- `patterns` 出现时整体替换默认有序列表；每项的 `match` 使用统一 `MatchRule`，通常是字符串 wildcard；
- `origin_mappings` 出现时整体替换，避免合并后产生不可解释的顺序；
- `variants` 不按静态身份键与 Model Driver 去重。对每个具体模型，先用 `provider_model_id` 在当前 Provider Rules 的 `variants` 中适配；只要命中至少一项，该模型的有效 variant 集合完全采用 Provider 结果；一项都未命中时，才回退到对应 Model Driver 的 `variants`；
- 字段缺失继续使用默认值；
- `{}` 仅用于 `custom` Provider：使用 Adapter 标准协议行为、保留原始模型名并搜索全部 Model Driver，不启用任何厂商映射。

配置不能把多个 API 代际合并为一个运行时探测或降级 Adapter。Cloud manifest 是完整的 cloud 来源版本，不是最终有效配置全集；它缺少的 Provider 身份可以继续由 builtin 提供。

内置专用 Provider 的常规渠道规则由 NDN 交付的 `.provider.json` 更新；Provider Instance 或调用方不得用任意 JSON 绕过该 catalog。代码中的核心执行逻辑不接受配置替换，但必须消费配置解析后的结果，不能另外硬编码同一份规则。

## 12. 已确定的实现约束

1. 首版 11 家内置 Provider 包括 OpenAI、Claude、Google Gemini、fal、OpenRouter、MiniMax、Kimi、GLM、DeepSeek、豆包和 Qwen；SN 作为独立扩展 Provider 保留。它们都必须在集成测试阶段进入对应的 T1/T1.5 和 T2 验收矩阵。
2. 配置型 Provider 只能使用运行时已经注册的 Protocol Adapter；用户只提供协议族和连接信息，接入测试自动解析并固化具体 Adapter。AICC 不开放第三方 Provider 插件或任意协议 ID。
3. Provider Rules、Model Driver、Pricing 和 Known Provider 保持独立对象和 revision；文件发现、下载、校验、替换及目标 seq 由 NDN 保证。AICC 在推理前或 Provider 定时库存刷新时统一收敛所有 applied seq 落后的 Provider；列表未变化且 seq 相同时只探测。
4. Provider variant 优先定义当前渠道中具体模型的 variant 集合及 adapter 参数 lowering；Provider 对该模型无任何 variant 命中时，才使用 Model Driver variant 及其默认 `provider_options`。Provider 可以增加、减少或替换 Model Driver 声明的 variant，不要求按 `model_driver + variant` 完整覆盖。
5. 旧 settings 中 `provider_driver` 承担的职责拆为实例级 `provider_profile_id`、`protocol_adapter_id` 和模型级 `model_driver_id`；新 settings 不兼容读取 `provider_driver`，但不因此删除 `buckyos-api` 和验收报告中已经导出的同名兼容字段。
6. OpenAI、Claude、Google Gemini 分别实现专用协议族；优先实现官方新接口。历史 API 代际由首个真实 Provider 需求触发实现，注册为协议族级共享 Adapter，后续 Provider 直接引用或通过 `base_adapter_id` 复用，不重复实现。
7. SN 使用独立 `sn-openai` Adapter，并以 `openai-responses` 为 `base_adapter_id`；支持 `api_key` 与 `dynamic_login` 两种认证模式。
8. 基础 Adapter 不依赖派生 Adapter。派生 Provider 的删除测试必须证明不需要修改基础 Adapter。
9. 官方 Profile 默认新接口；自定义 Provider 接入测试先测新接口，再测已注册的历史接口，用户不选择接口版本。解析完成后新旧 Adapter 不互相 fallback，只复用协议中立的底层组件。
10. Model Driver、Provider Rules、request/pricing rules 和发布 track 统一使用 `MatchRule`；简单规则保持 wildcard 字符串，多维条件才展开为对象，各业务模块不得再实现独立匹配 DSL。
11. 每个官方支持的 Provider（包括内置专用 Provider）必须提供独立 `.provider.json`；每个模型原厂必须提供独立 `.model.json`。Rust builtin 模块不得构造生产用 `ProviderRulesCatalog`、`KnownProviderCatalog` 或模型 metadata 作为第二真相源。
12. 未被官方支持的小型或自建代理可注册为 `custom` Provider，并用 `{}` 表示无渠道规则。其协议族只决定调用协议；模型归属必须按未改写的 `provider_model_id` 搜索全部 Model Driver，零命中 conservative fallback，多重命中拒绝。
13. 特殊 dialect 必须先尝试由 `.provider.json` 的有界声明表达；schema 不足时先评审统一 schema 扩展。只有无法安全声明化的 wire、认证、流式/任务状态机或错误语义才进入代码，并保持最小差异面。
