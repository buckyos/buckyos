# AICC 第一版内置 Provider 计划

版本：`v0.3-first-party-and-mainstream`
状态：Beta 2.2 目标规范

第一版直接支持以下 Provider，不再区分原文档中的四家 P0 和其它 P1：

```text
OpenAI / Claude / Gemini / fal / OpenRouter / MiniMax
Kimi / GLM / DeepSeek / 豆包（火山方舟）/ Qwen（阿里云百炼）
```

模块复用和实现入口以 [internal_module_architecture.md](internal_module_architecture.md) 为准。本文只定义首版覆盖范围和验收边界，不重复协议内部结构。

## 1. 范围原则

1. OpenAI、Claude、Gemini 是三个独立基础协议族，分别优先实现 Responses、Messages、Interactions；
2. Kimi、GLM 的首版官方主入口形成 Chat Completions 的真实需求；兼容厂商共用生态口径客户端 `legacy-openai-chat-completions`（基于官方口径的 `openai-chat-completions`），各家扩展参数写在 Provider Rules；OpenRouter 使用 `openai-responses` 派生实现；
3. MiniMax 文本接口复用生态口径客户端 `legacy-claude-messages`，只在派生层处理剩余的兼容差异；
4. DeepSeek、豆包、Qwen 优先复用 `openai-responses`，各自隔离扩展和限制，能用 Provider Rules 表达的不建派生客户端；
5. fal Queue 及各家媒体/异步接口保留原生 codec，只复用任务生命周期基础设施；
6. 同一历史接口只在首个真实需求出现时实现一次，后续 Provider 复用；
7. “首版支持 Provider”不等于无条件开放该厂商所有 API。只有已经映射到 AICC ApiType、进入 metadata 且通过协议合同的 operation 才进入库存；
8. 不使用本地模型；`agent.computer_use` 只对官方明确支持、metadata 已声明且协议合同已通过的模型开放，当前为 OpenAI GPT-5.6 系列。
9. “内置”只表示内置行为实现和发布验收，不表示把渠道规则硬编码进 Rust。每个官方支持的 Provider 必须有独立 `.provider.json`，每个模型原厂必须有独立 `.model.json`。
10. 特殊 dialect 优先通过 `.provider.json` 声明；只有无法安全声明化的 wire、认证、流式/任务状态机和错误语义才实现最小代码差异层。
11. 未被官方支持的小型 Provider 或用户自建代理使用 `custom + 显式选择的默认客户端 + {}` 接入；原始模型名必须直接匹配全部 Model Driver，系统不自动执行任何厂商前后缀或别名转换。

SN Provider 是扩展 Provider，不计入本次 11 个首版内置 Provider。SN 基于 LiteLLM，直接使用 OpenAI 族的标准客户端，不设专用客户端；API key/动态登录双鉴权是 SN 的 Provider 行为，继续与 OpenAI 基础实现隔离。

## 2. Provider 接入矩阵

| Provider ID | 首版主 Adapter | 首版定位 | Credential |
| --- | --- | --- | --- |
| `openai` | `openai-responses` + `openai-embeddings`、`openai-images`、`openai-audio`、`openai-videos` | 通用 LLM、embedding、image、audio、video，以及 GPT-5.6 computer-use | Bearer API key |
| `claude` | `claude-messages` | LLM、视觉理解 | `x-api-key` |
| `gemini` | `gemini-interactions` + `gemini-generate-content` | 多模态、embedding、image/audio/video | `x-goog-api-key` |
| `fal` | `fal-queue` | 图像、音频、视频生成或处理的长尾模型 | `Authorization: Key` |
| `openrouter` | `openrouter-responses -> openai-responses` + OpenAI 族标准客户端、`rerank`、`openrouter-native` | 聚合 LLM、长尾模型、成本/可用性路由 | Bearer API key |
| `minimax` | `legacy-claude-messages` + `minimax-media` | LLM、speech、image、video、music | named-header API key / 原生 Bearer（按客户端） |
| `kimi` | `legacy-openai-chat-completions -> openai-chat-completions` | LLM、视觉/视频理解 | Bearer API key |
| `glm` | `legacy-openai-chat-completions` + OpenAI 族标准客户端、`glm-video` | LLM、多模态、embedding | Bearer API key；可选短期 JWT |
| `deepseek` | `openai-responses` | LLM、推理、工具调用 | Bearer API key |
| `doubao` | `openai-responses` + `doubao-media`、`doubao-speech` | LLM、多模态、embedding、image/video | Bearer API key |
| `qwen` | `qwen-responses -> openai-responses` + `legacy-openai-chat-completions`、`qwen-media` | LLM、多模态、embedding、image/video/audio | Bearer API key |

Adapter 箭头表示语义上的子类/派生关系，不强制使用语言继承。没有真实 wire 差异时应直接引用基础 Adapter，删除空派生层。`+` 后面是 Known Provider 的 `extra_protocol_adapters`，由模型规则按 API 类型指向，见 [provider_profile_schema.md](provider_profile_schema.md) §2.1。

## 3. 首版最低验收

每个 Provider 至少完成：

- Known Provider/Profile 注册；
- credential schema 与脱敏；
- base_url/region/workspace 参数解析；
- 至少一项官方主 operation 的非流式调用；
- 主 operation 支持时的流式调用；
- 模型 discovery 或 catalog-only 库存构建；
- Provider Rules、Model Driver 和 operation 能力交集；
- 独立 `.provider.json`，以及涉及原厂模型时对应的独立 `.model.json`；
- dialect 差异的声明化评审，确认代码中没有可由 Provider Rules 表达的常规参数表；
- 官方错误到 AICC 稳定错误的映射；
- health probe、inventory LKGS、metadata applied seq；
- 禁用/删除/替换时停止刷新循环；
- 基础合同、厂商差异合同和 builtin 装配测试。

对异步媒体 Provider/operation 还必须完成 submit、poll/status、result、cancel、终态映射、超时和 TaskMgr bridge。仅有同步 LLM 能力的 Provider 不为形式完整而实现空任务接口。

## 4. 接口代际选择

| 协议族 | 首版主动实现 | 首版不预实现 |
| --- | --- | --- |
| OpenAI | Responses；因三家真实需求加入 Chat Completions（官方口径与生态口径两个客户端） | legacy Completions、Assistants 等无真实需求接口 |
| Claude | Messages（官方口径与生态口径两个客户端） | legacy Completions |
| Gemini | Interactions；`models.*` 方法族（`gemini-generate-content`），供兼容网关以及官方 Provider 的向量和长任务使用 | Live 等实时会话接口 |

内置 Provider 的客户端集合由 Known Provider 固定，不在运行时协商。自定义 Provider 接入时由用户显式选择默认客户端（例如 Responses 或 Chat Completions），系统不发探测请求，也不按“新接口优先”试选；实例再带上同一协议族里其余对自定义开放的客户端，结果固化到 Provider Instance。

认证、网络、限流和服务端故障不能被误判为“不支持新接口”并触发历史接口 fallback。

## 5. 模型发现与价格

价格事实的目标来源优先级：

```text
Provider 实时 discovery
  > Provider Rules 的渠道静态价格
  > unknown（无可靠渠道价格）
```

- OpenRouter Models API 的模型、能力和实时价格应进入动态 discovery；
- OpenAI、Claude、Gemini、MiniMax、Kimi 等存在官方模型机器接口时，复用相应 discovery parser；
- 所有 Provider 都先按其默认客户端所属协议族执行标准机器 discovery（`standard-models`），只有 OpenRouter、SN 的目录协议和方舟的 `status` 另有实现；发现不读取模型事实，模型事实只来自 Model Driver；如果厂商没有对应接口，该请求失败后才使用 Known Provider/Provider Rules/Model Driver 构建的静态 inventory；
- 运行时枚举合并后 catalog 中的全部 Known Provider；没有专用差异行为的新 Profile 复用已注册 Protocol Adapter 的标准 discovery，并自动合并 Provider Rules exact models 与明确引用的 Model Driver exact models 作为失败兜底，不维护厂商 ID 白名单；
- 禁止抓取官方文档网页、控制台页面或读取 SDK 私有列表模拟 discovery；
- Provider 实时价格属于实例动态事实，不写回静态 catalog；静态价格直接位于 Provider Rules，不单独建立 Pricing Catalog。

Model Driver 不保存价格或成本兜底，官方直连渠道也必须由 Provider 提供可靠价格；未知不等于免费。实际结算优先采用 Provider 响应费用，缺少适用价格或用量时保持 unknown。2026-09-25 已从全部 builtin Model Driver 配置删除 `model_pricing`，运行时的旧字段与 fallback 尚待 Review 后删除。

## 6. 能力开放规则

厂商文档声明的能力只有满足以下条件才可由 AICC 对外暴露：

```text
存在 typed AICC ApiType
+ 已实现对应 operation codec
+ Model Driver 声明稳定模型能力
+ Provider Rules 完成渠道映射
+ discovery/实例确认当前可用
+ contract test 通过
```

因此首版可以先让 11 家 Provider 都具备可用的主接口，再逐项增加媒体和专用 operation，而不需要在一个 Provider 文件中一次性实现厂商全部产品线。

## 7. 建议落地批次

### 批次 A：基础协议与直连 Provider

```text
openai-responses  -> OpenAI
claude-messages   -> Claude
gemini-interactions -> Gemini
```

### 批次 B：共享历史协议与派生 Provider

```text
openai-chat-completions
  -> OpenRouter
  -> Kimi
  -> GLM

claude-messages
  -> MiniMax

openai-responses
  -> DeepSeek
  -> 豆包
  -> Qwen
```

### 批次 C：异步与媒体

```text
fal Queue
OpenAI/Gemini 专用媒体 operation
MiniMax/GLM/豆包/Qwen 原生 operation
```

批次只表示实现依赖，不改变所有 11 家都属于第一版验收范围。当前验收 baseline 的缺口在编码完成后的集成测试阶段补齐：先完成 11 家及 SN 的 T1/T1.5，再完成 T2，最后进入 T3。

## 8. 非目标

- 不为未被具体 Provider 使用的历史 API 做预防性兼容；
- 不保证任意 OpenAI-compatible endpoint 自动可用；
- 不在基础 OpenAI/Claude/Gemini Adapter 中加入派生厂商分支；
- 不在运行时探测或协商 API 代际；自定义 Provider 的客户端由用户显式选择；
- 不把 Provider 专属任意 JSON 暴露到 AICC 公共请求；
- 不为每家 Provider 复制 HTTP、SSE、任务轮询、匹配器和 contract harness；
- 不在第一版实现本地模型；不对未经 metadata 和协议合同确认的模型泛化开放 `agent.computer_use`。
