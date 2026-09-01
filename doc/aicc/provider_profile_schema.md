# AICC Provider Profile 与 Provider Rules Schema

状态：Beta 2.2 目标规范
范围：定义 Provider Profile、Protocol Adapter、Provider Rules、Model Driver 与 Pricing 的稳定边界。AICC 重构必须以本文为准，不保留旧 `provider_driver` 兼容语义。

## 1. Provider 分为两类

AICC 不应把所有 Provider 都实现成同一种声明式配置。

### 1.1 内置专用 Provider

以下 Provider 应定制实现：

- OpenAI、Google Gemini、Anthropic 等原厂官方 Provider；
- OpenRouter 等影响力大、协议和模型规则具有明显特异性的聚合 Provider；
- 其他需要长期稳定支持、必须进入发布验收矩阵的 Provider。

专用 Provider 在程序中固定实现：

- 认证和 API 协议；
- 模型与价格 discovery；
- 原厂模型身份解析；
- moving alias 和 Provider variant 的处理；
- 不同模型、AICC `api_type` 与具体 operation 的选择；
- 同步、流式、异步任务和错误处理；
- 实时价格与 Provider inventory 的合并；
- discovery 失败后的 LKGS/default inventory 行为。

这些逻辑不作为外部 Provider 参数配置暴露。Provider API 升级时，由专用实现和测试一起升级，避免一份通用配置在 Provider 频繁变化后产生不稳定行为。

专用 Provider 仍然复用 Model Driver metadata，不复制模型 capabilities、逻辑挂载、家族、variants 和版本规则。

### 1.2 配置型 Provider

影响力较小、没有必要加入 AICC 内置实现，但能够复用现有协议 adapter 的 Provider，可以通过声明式配置接入。

配置型 Provider 适合处理：

- OpenAI-compatible 等已有协议的兼容服务；
- 模型名增加固定前后缀的代理服务；
- 只支持少量 Model Driver 的小型聚合平台；
- 少量模型需要指定不同 operation；
- Provider 无法查询实时价格，需要配置渠道默认价格。

如果一个 Provider 需要复杂状态机、特殊认证、新的流式格式、特殊错误恢复或大量模型专用分支，应升级为专用 Provider，而不是继续扩张配置 schema。

## 2. 配置文件原则

Provider 参数配置是配置型 Provider 内置默认行为的**可选覆盖层**，不是完整 Provider manifest。

- 程序已经确定的参数不写入配置；
- 所有字段均可省略；
- 空对象 `{}` 必须合法，表示全部使用对应 adapter 的默认方案；
- 实际调用始终使用 Provider discovery 返回的原始 `provider_model_id`；
- 尽量复用当前 Driver metadata 已有字段和规则结构；
- schema 只表达数据差异，不试图声明式实现完整 Provider Adapter。

以下内容由程序固定，不进入配置：

- protocol adapter 的具体实现；
- refresh interval；
- 无匹配时使用 conservative fallback；
- 多个 Model Driver 同时匹配时拒绝解析；
- discovery 失败策略；
- 配置加载、schema 解析，以及由 NDN `metadata_target_seq` 和 Provider `metadata_applied_seq` 驱动的全局库存收敛机制。

Provider Instance 的名称、凭据、区域和用户自定义 endpoint 属于实例私有配置，也不进入可云更新的 Provider 参数文件。

### 2.1 基础协议与派生 Adapter

OpenAI、Claude、Google Gemini 必须各自拥有专门实现、独立注册和独立验收的协议族。协议族不是可执行 Adapter；同一厂商的新旧 API 形态使用不同的内部 `protocol_adapter_id`，不能在一个 Adapter 内按 endpoint 能力或 Provider ID 切换。基础协议首先实现并维护官方推荐的新接口；历史接口不要求预先完整实现，只有首个真实 Provider 需要某个历史 API 代际时才增加对应 Adapter。这个 Adapter 属于协议族并可被所有兼容 Provider 复用，不属于首个触发需求的派生 Provider，也不能在后续 Provider 中重复实现。

内置厂商 Adapter 可以复用基础协议，并声明语义上的子类关系：

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
| `openai` | `openrouter-openai` | `openai-chat-completions` | OpenRouter 渠道扩展，复用其实际兼容的旧接口 |

新接口 Adapter 与兼容 Adapter 是平级实现。兼容 Adapter 不继承新接口 Adapter，也不通过调用新接口失败后回退旧接口。两者只允许复用低层、无状态且协议中立的组件，例如 HTTP transport、SSE framing、通用 JSON/错误工具和 AICC normalized IR；endpoint path、request schema、response event、错误映射和能力声明保持各自内聚。

同一个历史 API 代际只实现一份共享 Adapter。Provider 没有额外差异时，Provider Profile 或 Instance 直接保存这个 Adapter ID；确有渠道认证、endpoint 选择或错误语义差异时，才建立独立派生 Adapter，并用 `base_adapter_id` 指向共享历史 Adapter。多个派生 Adapter 可以引用同一个历史 Adapter，各自只实现差异层，不复制历史 wire protocol。

Provider Profile/Rules 必须在路由前得到一个确定的 Adapter 和 operation。Known Provider 由内置 Profile 固定该选择；用户添加 `custom` Provider 时只选择或识别 OpenAI、Claude、Gemini 等协议族，不选择 API 代际。接入测试按该协议族“官方新接口优先、运行时已注册的历史接口其次”的顺序验证，成功后把 resolved `protocol_adapter_id` 固化到 Provider Instance。接口不支持才继续测试下一候选；认证、网络和服务端故障必须直接报告，不能被误判成历史接口需求。运行时只使用已固化 Adapter，不重新探测，也不在一次调用中静默切换新旧 Adapter。

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

其他内置厂商也可以使用同样的派生 Adapter 模式，但必须有独立 ID、明确差异面和基础/派生两层验收。

## 3. Model Driver 与 Provider 配置边界

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
| 默认价格 | Provider 没有价格数据时使用的保守估值 |

Model Driver 的 variant 只定义语义身份，例如 `reasoning.high`。配置型 Provider 如何将它转换为请求参数，由 Provider 配置中的 `variants` 定义。

### 3.2 配置型 Provider 管理

| 用途 | 配置字段 |
| --- | --- |
| 限定参与匹配的 Model Driver metadata | `metadata_drivers` |
| Provider 厂商 slug 映射 | `origin_provider_aliases` |
| `provider_model_id` 到原厂身份的确定性映射 | `origin_mappings` |
| 渠道专属排除规则 | `models[].exclude` / `patterns[].exclude` |
| 选择按渠道模型名、原厂模型名或其它维度匹配 | `match: MatchRule`；字符串默认匹配渠道模型名 |
| Provider 请求参数 | `provider_options` / `variants` |
| 模型级请求默认值、改写和参数删除 | `request_rules` |
| Provider 渠道默认价格 | `pricing` |
| 按质量、尺寸、时长等请求维度计价 | `pricing.rules` |
| 模型使用的具体接口 | `operations` |
| Provider 无法提供的模型能力 | `remove_api_types` / `remove_features` |
| 渠道延迟和成本提示 | `estimated_latency_ms` / `latency_class` / `cost_class` |

Provider 配置只能收窄 Model Driver 声明的能力，不能增加模型固有能力。

## 4. 最小配置结构

```json
{}
```

需要覆盖默认行为时，配置只使用以下可选字段：

```json
{
  "metadata_drivers": [],
  "origin_provider_aliases": {},
  "origin_mappings": [],
  "models": [],
  "patterns": [],
  "variants": []
}
```

- `metadata_drivers`：参与匹配的 Model Driver 列表；省略时使用 adapter 的默认候选范围。
- `origin_provider_aliases`：Provider 命名中的厂商 slug 到 Model Driver 名称的映射。
- `origin_mappings`：可以从命名确定性解析原厂身份时使用的特殊映射。
- `models`：按完整 `provider_model_id` 精确匹配的 Provider 规则。
- `patterns`：有序 Provider 规则；每项的 `match` 通常直接写匹配完整 `provider_model_id` 的 wildcard 字符串，多维条件才写对象。
- `variants`：将 Model Driver 语义 variant 转换为 Provider 请求参数。

不增加 `refresh_interval_sec`、`on_no_match`、`on_ambiguous`、`failure_policy`、`protocol_adapter` 等程序固定字段。

`metadata_drivers` 显式为空数组表示不使用 Model Driver metadata，所有模型进入 conservative fallback；字段省略则使用 adapter 默认候选范围。

## 5. 模型规则

`models` 和 `patterns` 使用 [match_rule.md](match_rule.md) 定义的统一 `MatchRule`。简单规则只写字符串 wildcard；只有同时约束多个维度时才使用对象。exact `models` 优先；未命中 exact 时，`patterns` 按数组顺序使用第一条匹配规则。

专用 Provider 与配置型 Provider 都复用现有模型规则和 resolver。区别只是专用 Provider 从代码或内置数据提供 Provider model rules，配置型 Provider 从以下外部字段加载；专用 Provider 不因此开放外部覆盖。调用前可以产生临时的 resolved provider call，但它不是新的配置或真相源。

完整的可选配置项如下：

| 配置项 | 默认值 | 用途 | 来源 |
| --- | --- | --- | --- |
| `id` | 无 | `models` 中精确匹配模型；内部归一化为单维 `MatchRule` | 复用现有字段 |
| `match` | 无 | `patterns` 中的 `MatchRule`；通常直接写 wildcard 字符串 | 统一字段 |
| `exclude` | `false` | 从当前 Provider inventory 排除模型 | 从 Model Driver metadata 移入 |
| `operations` | `{}` | method/api_type 到 adapter operation 的映射 | 新增 |
| `provider_options` | `{}` | 调用该模型时附加的 Provider 参数 | 从 Model Driver metadata 移入 |
| `request_rules` | `[]` | 请求默认值、条件改写和不兼容参数删除 | 新增 |
| `pricing` | 无 | Provider 渠道价格及条件价格规则 | 从 Model Driver metadata 移入并扩展 |
| `remove_api_types` | `[]` | 删除当前 Provider 无法提供的 API type | 新增 |
| `remove_features` | `[]` | 删除当前 Provider 无法提供的 feature | 新增 |
| `estimated_latency_ms` | 无 | 渠道默认延迟估计 | 从 Model Driver metadata 移入 |
| `latency_class` | 无 | 渠道延迟分类 | 从 Model Driver metadata 移入 |
| `cost_class` | 无 | 渠道成本分类 | 从 Model Driver metadata 移入 |

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

### 5.3 Request rules

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

### 5.4 Pricing

`pricing` 保留现有 token 价格字段，并补充非 token 计价：

- `currency`；
- `input_token`、`output_token`、`cache_input_token`；
- `estimated_cost`：无法精确计算时的默认估值；
- `unit`：`request`、`image`、`audio_second` 或 `video_second`；
- `amount`：对应 unit 的单价；
- `rules`：根据请求参数选择单价的有序规则，使用与 `request_rules.when` 相同的 `MatchRule`。

`pricing.rules` 使用第一条命中的价格；均未命中时使用外层 `amount` 或 `estimated_cost`。例如 GPT Image 按 quality/size 计价：

```json
{
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
```

image 单价自动乘以归一化请求中的生成数量；audio/video second 单价自动乘以归一化时长。

### 5.5 能力收窄

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
> Provider 配置 models / patterns 中的价格
> Model Driver 默认价格
```

Provider 配置中的价格不能覆盖更新鲜的实时价格。

## 8. OpenAI 官方 Provider 示例

OpenAI 是内置专用 Provider。它在程序中固定只使用 OpenAI Model Driver，并固定实现 discovery、operation 选择和调用协议，因此不需要 Provider 参数文件。

如果统一加载流程要求配置对象存在，其内容为：

```json
{}
```

`metadata_drivers: ["openai"]` 是 OpenAI 专用实现的内置约束，不需要在外部配置中重复。

## 9. OpenRouter 示例

OpenRouter 是内置专用 Provider，而不是配置型 Provider。

以下逻辑由 OpenRouter 实现固定，并纳入发布测试：

- 解析 `vendor/model` 命名并映射到候选 Model Driver；
- 维护 OpenRouter vendor slug 与 Model Driver 的别名关系；
- 排除 moving alias、Provider variant alias 和 OpenRouter 虚拟模型；
- 保留原始 `provider_model_id` 完成实际调用；
- 按模型和 AICC `api_type` 选择 OpenRouter chat、image、video 等 operation；
- 从 OpenRouter discovery 获取价格并覆盖 Model Driver 默认价格；
- 对 OpenRouter API 升级进行兼容、测试和版本发布。

它不通过外部配置暴露上述规则。统一配置对象为空：

```json
{}
```

OpenRouter 仍从 OpenAI、Claude、Gemini 等 Model Driver metadata 获取模型固有能力，但候选范围和命名解析由专用实现决定。

## 10. 小型兼容 Provider 示例

假设 `example-router` 是影响力较小、仍只提供 Chat Completions 的 OpenAI-compatible 聚合服务。用户只声明 `openai` 协议族，接入测试将其解析为已注册的 `openai-chat-completions`；Provider 规则只需要配置模型匹配范围、命名映射和少量 operation：

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

如果接入测试解析出的 Adapter 的全部默认行为已经适用，该 Provider 同样允许使用空配置：

```json
{}
```

## 11. 默认值与覆盖语义

配置型 Provider 解析出的特定 Adapter 提供经过验证的默认行为，外部配置只覆盖显式字段；配置不能把多个 API 代际合并为一个运行时探测或降级 Adapter：

- map 按 key 覆盖；
- `models` 按 `id` 覆盖同名 exact rule；
- `patterns` 出现时整体替换默认有序列表；每项的 `match` 使用统一 `MatchRule`，通常是字符串 wildcard；
- `origin_mappings` 出现时整体替换，避免合并后产生不可解释的顺序；
- `variants` 按 `model_driver + variant + match` 覆盖；
- 字段缺失继续使用默认值；
- `{}` 完全使用默认方案。

内置专用 Provider 不接受外部规则覆盖其核心调用和解析逻辑。

## 12. 已确定的实现约束

1. 内置专用 Provider 包括 OpenAI、Claude、Google Gemini、OpenRouter、SN、MiniMax 和 fal；新增发布级 Provider 必须进入协议验收矩阵。
2. 配置型 Provider 只能使用运行时已经注册的 Protocol Adapter；用户只提供协议族和连接信息，接入测试自动解析并固化具体 Adapter。AICC 不开放第三方 Provider 插件或任意协议 ID。
3. Provider Rules、Model Driver、Pricing 和 Known Provider 保持独立对象和 revision；文件发现、下载、校验、替换及目标 seq 由 NDN 保证。AICC 在推理前或 Provider 定时库存刷新时统一收敛所有 applied seq 落后的 Provider；列表未变化且 seq 相同时只探测。
4. Model Driver variant 定义语义身份；Provider variant 必须完整覆盖该身份到 adapter 参数的 lowering，否则该 Provider 不得声明对应 variant 可用。
5. 旧 `provider_driver` 拆为 `provider_profile_id`、`protocol_adapter_id` 和模型级 `model_driver_id`，不提供兼容读取。
6. OpenAI、Claude、Google Gemini 分别实现专用协议族；优先实现官方新接口。历史 API 代际由首个真实 Provider 需求触发实现，注册为协议族级共享 Adapter，后续 Provider 直接引用或通过 `base_adapter_id` 复用，不重复实现。
7. SN 使用独立 `sn-openai` Adapter，并以 `openai-responses` 为 `base_adapter_id`；支持 `api_key` 与 `dynamic_login` 两种认证模式。
8. 基础 Adapter 不依赖派生 Adapter。派生 Provider 的删除测试必须证明不需要修改基础 Adapter。
9. 官方 Profile 默认新接口；自定义 Provider 接入测试先测新接口，再测已注册的历史接口，用户不选择接口版本。解析完成后新旧 Adapter 不互相 fallback，只复用协议中立的底层组件。
10. Model Driver、Provider Rules、request/pricing rules 和发布 track 统一使用 `MatchRule`；简单规则保持 wildcard 字符串，多维条件才展开为对象，各业务模块不得再实现独立匹配 DSL。
