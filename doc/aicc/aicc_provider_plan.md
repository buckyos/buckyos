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
2. OpenRouter、Kimi、GLM 的首版官方主入口形成 `openai-chat-completions` 的真实需求，三者共享一份基础实现；
3. MiniMax 文本接口优先复用 `claude-messages`，只在派生层处理兼容差异；
4. DeepSeek、豆包、Qwen 优先复用 `openai-responses`，各自隔离扩展和限制；
5. fal Queue 及各家媒体/异步接口保留原生 codec，只复用任务生命周期基础设施；
6. 同一历史接口只在首个真实需求出现时实现一次，后续 Provider 复用；
7. “首版支持 Provider”不等于无条件开放该厂商所有 API。只有已经映射到 AICC ApiType、进入 metadata 且通过协议合同的 operation 才进入库存；
8. 不使用本地模型，不纳入 `agent.computer_use`。

SN Provider 的既有 `sn-openai -> openai-responses` 设计保持不变，但它是扩展 Provider，不计入本次 11 个首版内置 Provider。SN 的 API key/动态登录双鉴权继续与 OpenAI 基础实现隔离。

## 2. Provider 接入矩阵

| Provider ID | 首版主 Adapter | 首版定位 | Credential |
| --- | --- | --- | --- |
| `openai` | `openai-responses` + 专用 operation | 通用 LLM、embedding、image、audio、video | Bearer API key |
| `claude` | `claude-messages` | LLM、视觉理解 | `x-api-key` |
| `gemini` | `gemini-interactions` + Gen Media | 多模态、embedding、image/audio/video | `x-goog-api-key` |
| `fal` | `fal-queue` | 图像、音频、视频生成或处理的长尾模型 | `Authorization: Key` |
| `openrouter` | `openrouter-openai -> openai-chat-completions` | 聚合 LLM、长尾模型、成本/可用性路由 | Bearer API key |
| `minimax` | `minimax-messages -> claude-messages` + 原生媒体 | LLM、speech、image、video、music | named-header API key / 原生 Bearer（按 operation） |
| `kimi` | `kimi-chat -> openai-chat-completions` | LLM、视觉/视频理解 | Bearer API key |
| `glm` | `glm-chat -> openai-chat-completions` + 原生异步 | LLM、多模态、embedding | Bearer API key；可选短期 JWT |
| `deepseek` | `deepseek-responses -> openai-responses` | LLM、推理、工具调用 | Bearer API key |
| `doubao` | `doubao-responses -> openai-responses` + 原生媒体 | LLM、多模态、embedding、image/video | Bearer API key |
| `qwen` | `qwen-responses -> openai-responses` + 原生媒体 | LLM、多模态、embedding、image/video/audio | Bearer API key |

Adapter 箭头表示语义上的子类/派生关系，不强制使用语言继承。没有真实 wire 差异时应直接引用基础 Adapter，删除空派生层。

## 3. 首版最低验收

每个 Provider 至少完成：

- Known Provider/Profile 注册；
- credential schema 与脱敏；
- endpoint/region/workspace 参数解析；
- 至少一项官方主 operation 的非流式调用；
- 主 operation 支持时的流式调用；
- 模型 discovery 或 catalog-only 库存构建；
- Provider Rules、Model Driver 和 operation 能力交集；
- 官方错误到 AICC 稳定错误的映射；
- health probe、inventory LKGS、metadata applied seq；
- 禁用/删除/替换时停止刷新循环；
- 基础合同、厂商差异合同和 builtin 装配测试。

对异步媒体 Provider/operation 还必须完成 submit、poll/status、result、cancel、终态映射、超时和 TaskMgr bridge。仅有同步 LLM 能力的 Provider 不为形式完整而实现空任务接口。

## 4. 接口代际选择

| 协议族 | 首版主动实现 | 首版不预实现 |
| --- | --- | --- |
| OpenAI | Responses；因三家真实需求加入 Chat Completions | legacy Completions、Assistants 等无真实需求接口 |
| Claude | Messages | legacy Completions |
| Gemini | Interactions | `generateContent`，除非某个已选 operation 确认只能使用它 |

内置 Provider 的 Adapter 选择由 Known Provider 固定，不在运行时协商。自定义 Provider 接入时只要求用户识别协议族；系统先测试该协议族官方新接口，再测试已经注册的历史接口。用户不选择 API 版本，成功结果固化到 Provider Instance。

认证、网络、限流和服务端故障不能被误判为“不支持新接口”并触发历史接口 fallback。

## 5. 模型发现与价格

价格事实的来源优先级遵循现有设计：

```text
Provider 实时 discovery
  > Provider Rules 的渠道静态价格
  > Model Driver 的保守成本估值
```

- OpenRouter Models API 的模型、能力和实时价格应进入动态 discovery；
- OpenAI、Claude、Gemini、MiniMax、Kimi 等存在官方模型机器接口时，复用相应 discovery parser；
- 其它 Provider 如果没有稳定的官方机器接口，使用 Known Provider/Provider Rules/Model Driver 构建 catalog-only inventory；
- 禁止抓取官方文档网页、控制台页面或读取 SDK 私有列表模拟 discovery；
- Provider 实时价格属于实例动态事实，不写回静态 catalog；静态价格直接位于 Provider Rules，不单独建立 Pricing Catalog。

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

批次只表示实现依赖，不改变所有 11 家都属于第一版验收范围。

## 8. 非目标

- 不为未被具体 Provider 使用的历史 API 做预防性兼容；
- 不保证任意 OpenAI-compatible endpoint 自动可用；
- 不在基础 OpenAI/Claude/Gemini Adapter 中加入派生厂商分支；
- 不让用户配置 API 代际；
- 不把 Provider 专属任意 JSON 暴露到 AICC 公共请求；
- 不为每家 Provider 复制 HTTP、SSE、任务轮询、匹配器和 contract harness；
- 不在第一版实现本地模型或 `agent.computer_use`。
