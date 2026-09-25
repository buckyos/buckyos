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

专用 Provider 仍然必须使用本厂商独立的 `.provider.json`，并复用 Model Driver metadata。Provider variant 转换、operation、请求参数差异、能力收窄和静态价格写入 `.provider.json`。身份特例和已确认的 moving alias 由可选 `match_model_driver` 实现，不再配置 origin 变换 DSL。只有统一配置 schema 无法安全表达的个性逻辑才允许留在代码中。

### 1.2 配置型 Provider

复用客户端现有 Protocol Adapter、且不需要专用 discovery、签名认证、动态登录或任务状态机的渠道，应作为配置型 Provider 接入。配置型 Provider 可以是云 catalog 正式发布的独立 Provider Profile，也可以是用户自行添加的 `custom` Provider。典型情况是兼容现有协议的厂商、小型 Provider 或用户自建代理。

配置型 Provider 适合处理：

- OpenAI-compatible 等已有协议的兼容服务；
- 模型名增加固定前后缀的代理服务；
- 只支持少量 Model Driver 的小型聚合平台；
- 少量模型需要指定不同 operation；
- Provider 无法查询实时价格，但有已核实且适用的渠道静态价格时，可在 Provider Rules 中声明；否则保持 unknown。

正式发布的配置型 Provider 由有效 Known Provider 和 Provider Rules 装配。静态库存仅来自 `static_inventory_models` 或实例明确配置的 discovery snapshot；`models[]` 技术规则和 Model Driver 全量定义均不是上线清单。默认库存随当前有效 catalog 刷新。发现失败的显式 fallback 标为 Degraded，认证和整份响应格式错误必须报告。

`custom` 无厂商特例，统一在全部 Model Driver 精确 ID 集合中匹配。`gpt-5.6-sol` 完全匹配；带命名空间或日期的 ID 可受限包含匹配。未命中或歧义只隔离该模型并列入 unmatched，不生成猜测能力。

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
- 无匹配时记录 unmatched，不进入库存；
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

| `protocol_family_id` | `protocol_adapter_id` | `base_adapter_id` | 定位 |
| --- | --- | --- | --- |
| `openai` | `openai-responses` | 无 | OpenAI 官方默认的新接口实现 |
| `openai` | `openai-chat-completions` | 无 | 首个真实需求出现时才注册，之后由兼容 Provider 共享的 Chat Completions 实现 |
| `openai` | `openai-completions` | 无 | 首个真实需求出现时才注册，之后由兼容 Provider 共享的旧 Text Completions 实现 |
| `claude` | `claude-messages` | 无 | Claude 官方默认 Messages 实现 |
| `claude` | `claude-completions` | 无 | 按首次真实需求实现，之后在协议族内共享 |
| `gemini` | `gemini-interactions` | 无 | Gemini 官方默认的新接口实现 |
| `gemini` | `gemini-generate-content` | 无 | 按首次真实需求实现，之后在协议族内共享 |
| `openai` | `sn-openai` | `openai-responses` | SN 鉴权扩展，当前复用 Responses 实现 |
| `openai` | `openrouter-responses` | `openai-responses` | OpenRouter 渠道扩展，复用 OpenResponses 兼容接口 |

新接口 Adapter 与兼容 Adapter 是平级实现。兼容 Adapter 不继承新接口 Adapter，也不通过调用新接口失败后回退旧接口。两者只允许复用低层、无状态且协议中立的组件，例如 HTTP transport、SSE framing、通用 JSON/错误工具和 AICC normalized IR；endpoint path、request schema、response event、错误映射和能力声明保持各自内聚。

同一个历史 API 代际只实现一份共享 Adapter。Provider 没有额外差异时，Provider Profile 或 Instance 直接保存这个 Adapter ID。确有渠道差异时，先用 `.provider.json` 的 operation、provider options、request rules、能力收窄及其它受限声明表达；只有不同 wire envelope、流事件状态机、签名/动态认证算法、任务生命周期或无法声明化的错误语义，才建立独立派生 Adapter，并用 `base_adapter_id` 指向共享历史 Adapter。多个派生 Adapter 可以引用同一个历史 Adapter，各自只实现剩余的最小逻辑差异，不复制历史 wire protocol，也不在代码中保存可由 Provider Rules 表达的参数表。

Provider Profile/Rules 必须在路由前得到一个确定的 Adapter 和 operation。内置 Provider 的默认选择由 Known Provider catalog 和 `.provider.json` 固定；用户添加 `custom` Provider 时只选择或识别 OpenAI、Claude、Gemini 等协议族，不选择 API 代际。接入测试只枚举无 `base_adapter_id` 且声明 `probe_path` 的协议族级 Adapter，按“官方新接口优先、运行时已注册的历史接口其次”的 `probe_priority` 顺序发送带认证的空 JSON 探测，成功识别后把 resolved `protocol_adapter_id` 固化到 Provider Instance。只有 `404/405/501` 继续下一候选；其它 4xx 证明接口存在，认证、限流、网络和 5xx 直接报告，不能被误判成历史接口需求。运行时只使用已固化 Adapter，不重新探测，也不在一次调用中静默切换新旧 Adapter。

一个 Provider 需要多套 protocol client 时，不把实例字段扩成无约束的 Adapter 数组。
Adapter 是该 Provider 的可执行协议边界，可以注册多个按 operation 分派的
`OperationCodec`/`NativeTaskCodec`，每个 codec 可委托不同的基础协议实现。MiniMax 的
Messages、T2A、图片、音乐和视频即按此方式组合；OpenRouter 的 Responses 与 rerank 也按
operation 显式组合。这样仍满足“Provider 可持有多个 protocol client”，同时保证一次路由
在调用前得到唯一的 `adapter + operation + codec`，避免运行时试探或静默切换。

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

行为 registry 以稳定 behavior ID 注册 discovery、动态登录或其它不可声明执行行为，不是 Provider Profile 白名单。Known Provider 必须在 metadata 中选择已注册 discovery behavior；通用 OpenAI-compatible/Claude/Gemini discovery 也有稳定 ID，可由任意新 Profile 复用。refresh 时从当前 Provider Rules 的 `static_inventory_models` 重建显式 default inventory。任何未知或冲突 behavior 均拒绝装配，不允许读取 `ui_hints`、按 Provider ID 猜测或静默 first-match。

可选 credential 由 typed `credential_variants[]` 声明，实例在 `auth.mode=api_key` 时用 `credential_kind` 显式选择；省略则使用 `credential` 默认值。区域入口由 typed `connection.region_base_urls` 声明，只有实例未显式提供 `base_url` 时才按解析后的 region 选择。GLM 的 `glm_jwt` 和 GLM/MiniMax 的区域入口均通过这两个 typed 字段进入 production registry。SN 的 `device_jwt` 是 SN 登录实现支持的稳定行为 ID，由显式 `auth.login_profile` 选择和校验，不从可选的 `ui_hints` 推断。

### 3.1 Model Driver metadata 管理

Model Driver v2 声明精确模型 ID、API/能力、LLM 规格和 supported_efforts。版本由官方 ID 的数值元组独立排序；无价格、参数模板或可用性。有限 pattern 在编译时展开为精确 ID，通配模型成员关系被拒绝。

### 3.2 Provider Rules 管理

| 字段 | 职责 |
| --- | --- |
| `static_inventory_models` | 明确的渠道库存，不从技术/价格规则推导 |
| `supplemental_inventory_api_types` | 动态查询未覆盖、允许静态补充的 API 集合 |
| `models` / `patterns` | operation、参数、排除、能力收窄 |
| `variants` | 渠道可准确执行的参数映射，与 Model Driver supported_efforts 求交 |
| `model_pricing` | 渠道价格；可按模型和 region/workspace/account 匹配 |
| `reported_cost` | 自报实际费用的 currency 和 `total_request_cost` 语义 |

native 使用 base；其他预设为 `reasoning-{effort}`，包括 minimal 和 thinking。模型侧不再重复定义 variants。非法 exact/cached 预设被拒绝；无映射的合法 effort 进入 unavailable_presets。调用参数不能覆盖已选定预设。

### 3.3 fal 的临时模型归属

当前 `fal-ai/esrgan`、`fal-ai/imageutils/rembg`、`fal-ai/deepfilternet3` 和 `fal-ai/video-upscaler` 尚无对应原厂 Provider 接入，暂时统一归属 `fal` Model Driver。其它配置随 fal 官方事实调整。

## 4. Custom Provider 的最小规则

省略厂商规则时仍可使用通用身份匹配。人工修正使用实例：

```json
{"instance_rules":{"model_driver_overrides":{"stable/gpt-5-6":"openai/gpt-5.6"},"exclude_models":[]}}
```

目标必须是存在的精确 `driver/model_id`。无效 override 是终止失败，不继续通用匹配。实例和 Provider 主动排除不算 unmatched。

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
| `latency_class` | 无 | 渠道延迟分类 | 从 Model Driver metadata 移入 |
| `cost_class` | 无 | 渠道成本分类 | 从 Model Driver metadata 移入 |

`model_pricing` 只属于 Provider Rules，是与 `models` / `patterns` 并列的顶层数组，不属于单个模型规则。把价格从模型
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

`model_driver` 不属于 Provider 模型规则。它是 Model Driver 唯一匹配后的解析结果；Provider 不限定 driver 白名单；可选 matcher 返回 Matched / NotHandled / Failed，成功目标仍须精确校验。

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

价格只在 Provider Rules 中声明，放在与 `models` / `patterns` 并列的顶层 `model_pricing` 表中，每项只允许 `id` 与 `match`
二选一：

```json
{
  "model_pricing": [
    { "id": "vendor/image-model", "pricing": { "currency": "USD", "unit": "image", "amount": 0.042 } },
    { "match": "vendor/fast-*", "pricing": { "currency": "USD", "input_token": 5e-07, "output_token": 2e-06 } }
  ]
}
```

解析顺序固定为：以 `provider_model_id` 先查精确 `id`，未命中再按声明顺序取第一条命中的 `match`。
Model Driver 不参与价格查找。查找发生在技术规则解析
之后，与模型本身命中 `models` 还是 `patterns` 无关，因此通配技术规则与逐模型价格可以并存。

`pricing` 本身支持三类正交的计量方式：

- token 计量：`currency` 与 `input_token` / `output_token` / `cache_input_token`，都是**每 token**
  单价（官方"元/百万 Tokens"要除以 1e6）；
- 非 token 计量：`unit` + `amount`，`unit` 取值 `request`、`image`、`audio_second`、
  `video_second`、`character`、`second`（算力秒）、`megapixel`（百万像素）；同一份 `pricing` 内
  token 字段与 `unit` 互斥；
- `estimated_cost`：仅在有可靠渠道依据时显式声明的估值，只作展示与路由参考，不能用于补造缺失价格或实际账单。

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
  Provider Rules 专属能力；`tiers` 与 `time_windows` 同样只在 Provider 价格中声明。
  Model Driver 不接受任何 `model_pricing`，而不只是拒绝其中的条件规则。

以下仅示意 Provider 按 quality/size 计价的结构，金额不是已核验的实际报价：

```json
{
  "model_pricing": [
    {
      "id": "vendor/image-model",
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

实例 model_driver_overrides → Provider matcher → 完全匹配 → 忽略大小写的最长受限包含 → unmatched。匹配段前须为字符串开头或非字母数字，末尾只允许空或日期形状后缀；同长候选为歧义。Provider Failed 和无效 Matched 均不得继续通用匹配。已知浮动别名未确认版本或缺 metadata 时使用 unresolved_alias。

匹配成功后求交模型事实、Adapter 支持和真实渠道限制，再应用参数/价格映射并生成合法预设。warning 对未变化的 `(instance, model, reason)` 去重。`list_providers.inventory` 暴露 `unmatched_models`、`unavailable_presets`、`unpriced_models`，刷新响应增加 `unmatched_count`。

## 7. 价格优先级

目标价格优先级由程序固定：

```text
Provider 实时 discovery 价格
> Provider Rules 的 model_pricing（渠道价）
> unknown（无可靠渠道价格）
```

Provider Rules 的静态价格不能覆盖更新鲜且适用于本次调用的实时价格。Model Driver 不参与
价格解析，不提供原厂默认价或保守估值兜底；官方直连 Provider 也遵守这一边界。没有匹配价格
时保持 unknown，不填 0、不视为免费、不从型号或其他渠道推测。只有确认了渠道、币种和计费
条件的价格才能写入 Provider Rules，不能把删除的 Model Driver 价表直接搬过去。

结算时 Provider 响应中的实际费用优先；没有实际费用且缺少完整适用的价格与用量时，费用仍为
unknown，不得标记为财务完整。估值与实际费用必须区分。

2026-09-25 已实现：统一 cache/模态 usage，缺失费率与超档返回 unknown，自报费用必须声明币种。价格含 source_url/verified_at，异常比例需要 ratio_exception；动态价格和 catalog 使用相同校验。详见 [Provider 升级记录](provider_upgrade_implementation.md)。

## 8. OpenAI 官方 Provider 示例

OpenAI 是官方内置专用 Provider。程序固定实现 discovery 和调用协议，同时必须提供独立 `openai.provider.json`，明确限定原厂 metadata 和 operation，例如：

```json
{
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

示例只展示边界，不取代完整 operation 表。OpenAI 官方 Provider 不使用 `{}` 的 custom 语义，operation 使用有效配置中的明确映射。

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

## 10. 特殊命名和浮动别名

OpenRouter matcher 解析 vendor/model，anthropic → claude、google → gemini、moonshotai → kimi、z-ai → glm，然后校验模型 ID。无法绑定的浮动/变体调用名保留在 discovery 并产生 unresolved_alias，不静默过滤。

DeepSeek 官方旧 V4 Flash 调用名已重定向，V4.1 metadata 尚未接入时隔离为 unresolved_alias；其它 Provider 的固定 V4 Flash 身份不受影响。自定义网关需要点号/连字符归一化时，实现局部 matcher 或配置精确 instance override；不更改全局匹配语义。

## 11. 文件选择与规则解析语义

来源选择先于规则解析。同一 `provider_profile_id` 在多个来源出现时，只读取 `system-config > local > cloud > builtin` 中最高优先级的完整文件；下层同名文件不参与解析，也不提供缺失字段的默认值。下面的覆盖规则只用于“已选文件内部的规则”和程序定义的 schema/Adapter 默认值，不是跨来源 merge：

- map 按 key 覆盖；
- `models` 按 `id` 覆盖同名 exact rule；
- `patterns` 出现时整体替换默认有序列表；每项的 `match` 使用统一 `MatchRule`，通常是字符串 wildcard；
- matcher 返回的失败不允许后续通用匹配；
- v1 的 `variants` 不按静态身份键与 Model Driver 去重：具体模型命中 Provider Rules 时完全采用 Provider 集合，否则使用 Model Driver `variants`。LLM v2 目标改为由 `supported_efforts` 派生语义身份，匹配的 Provider 集合只能收窄它；没有 Provider variant 匹配时使用 Adapter 标准转换，不再回退到 Model Driver 参数模板（见 §3.1）；
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
10. Provider Rules、request/pricing rules 和发布 track 复用 `MatchRule`；Model Driver 成员关系只允许有限精确 ID，不用通配符引入未知模型。
11. 每个官方支持的 Provider（包括内置专用 Provider）必须提供独立 `.provider.json`；每个模型原厂必须提供独立 `.model.json`。Rust builtin 模块不得构造生产用 `ProviderRulesCatalog`、`KnownProviderCatalog` 或模型 metadata 作为第二真相源。
12. 未被官方支持的小型或自建代理可注册为 `custom` Provider，并用 `{}` 表示无渠道规则。其协议族只决定调用协议；模型归属必须通过统一匹配链搜索全部 Model Driver，零命中和多重命中均记录 unmatched，只隔离该模型。
13. 特殊 dialect 必须先尝试由 `.provider.json` 的有界声明表达；schema 不足时先评审统一 schema 扩展。只有无法安全声明化的 wire、认证、流式/任务状态机或错误语义才进入代码，并保持最小差异面。
