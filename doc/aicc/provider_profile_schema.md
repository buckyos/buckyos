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
- 配置加载、校验、云更新、activation 和回滚机制。

Provider Instance 的名称、凭据、区域和用户自定义 endpoint 属于实例私有配置，也不进入可云更新的 Provider 参数文件。

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
| 选择按渠道模型名还是原厂模型名匹配 | `match_source` |
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
- `patterns`：按完整 `provider_model_id` 匹配的有序规则，按数组顺序从精确到宽松处理。
- `variants`：将 Model Driver 语义 variant 转换为 Provider 请求参数。

不增加 `refresh_interval_sec`、`on_no_match`、`on_ambiguous`、`failure_policy`、`protocol_adapter` 等程序固定字段。

`metadata_drivers` 显式为空数组表示不使用 Model Driver metadata，所有模型进入 conservative fallback；字段省略则使用 adapter 默认候选范围。

## 5. 模型规则

`models` 和 `patterns` 复用已有 exact/pattern 规则结构。exact `models` 优先；未命中 exact 时，`patterns` 按数组顺序使用第一条匹配规则。

专用 Provider 与配置型 Provider 都复用现有模型规则和 resolver。区别只是专用 Provider 从代码或内置数据提供 Provider model rules，配置型 Provider 从以下外部字段加载；专用 Provider 不因此开放外部覆盖。调用前可以产生临时的 resolved provider call，但它不是新的配置或真相源。

完整的可选配置项如下：

| 配置项 | 默认值 | 用途 | 来源 |
| --- | --- | --- | --- |
| `id` | 无 | `models` 中精确匹配模型 | 复用现有字段 |
| `pattern` | 无 | `patterns` 中 wildcard 匹配模型 | 复用现有字段 |
| `match_source` | `provider_model_id` | 选择规则匹配渠道模型名或 `origin_model_id` | 新增 |
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
  "pattern": "vendor/veo-3.1-*",
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

`match_source` 只允许：

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

`when` 可以是单个谓词，也可以是 `{ "all": [...] }` 表示多个谓词同时成立。单个谓词只包含 `path`、`op`、`value`；第一版只支持 `exists`、`equals`、`not_equals`、`in`、`contains`。`all` 只接受一层谓词数组，不递归嵌套，也不支持脚本、任意表达式或自定义函数。

以下规则可以替代 GPT nano 默认参数和 GPT/Codex sampling 参数特判：

```json
{
  "pattern": "gpt-5-nano*",
  "match_source": "origin_model_id",
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
        "path": "/reasoning/effort",
        "op": "not_equals",
        "value": "none"
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
- `rules`：根据请求参数选择单价的有序规则，使用与 `request_rules.when` 相同的谓词。

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
          "all": [
            {
              "path": "/quality",
              "op": "equals",
              "value": "high"
            },
            {
              "path": "/size",
              "op": "in",
              "value": [
                "1536x1024",
                "1024x1536"
              ]
            }
          ]
        },
        "amount": 0.167
      },
      {
        "when": {
          "path": "/quality",
          "op": "equals",
          "value": "low"
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
> Provider Instance 显式价格 override
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

假设 `example-router` 是影响力较小的 OpenAI-compatible 聚合服务，可以复用已有通用 adapter，只需要配置模型匹配范围、命名映射和少量 operation：

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
      "match": {
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
      "pattern": "*:*",
      "exclude": true
    },
    {
      "pattern": "*/*latest*",
      "exclude": true
    },
    {
      "pattern": "openai/gpt-5*",
      "operations": {
        "llm": "chat.completions.create"
      }
    }
  ]
}
```

如果通用 adapter 的全部默认行为已经适用，该 Provider 同样允许使用空配置：

```json
{}
```

## 11. 默认值与覆盖语义

配置型 Provider 的通用 adapter 提供经过验证的默认行为，外部配置只覆盖显式字段：

- map 按 key 覆盖；
- `models` 按 `id` 覆盖同名 exact rule；
- `patterns` 出现时整体替换默认有序列表；
- `origin_mappings` 出现时整体替换，避免合并后产生不可解释的顺序；
- `variants` 按 `model_driver + variant + model_pattern` 覆盖；
- 字段缺失继续使用默认值；
- `{}` 完全使用默认方案。

内置专用 Provider 不接受外部规则覆盖其核心调用和解析逻辑。

## 12. 已确定的实现约束

1. 内置专用 Provider 包括 OpenAI、Claude、Google Gemini、OpenRouter、SN、MiniMax 和 fal；新增发布级 Provider 必须进入协议验收矩阵。
2. 配置型 Provider 只能选择运行时已经注册的 Protocol Adapter；AICC 不开放第三方 Provider 插件或任意协议 ID。
3. Provider Rules、Model Driver、Pricing 和 Known Provider 使用统一 catalog 更新、严格校验、原子 activation 与 LKGS 机制，但保持独立对象和 revision。
4. Model Driver variant 定义语义身份；Provider variant 必须完整覆盖该身份到 adapter 参数的 lowering，否则该 Provider 不得声明对应 variant 可用。
5. 旧 `provider_driver` 拆为 `provider_profile_id`、`protocol_adapter_id` 和模型级 `model_driver_id`，不提供兼容读取。
