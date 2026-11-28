# AICC Provider 协议客户端拆分 TODO

日期：2026-11-27。状态：立项，代码未实施；Q1 到 Q16、N1、N2 owner 已于当天全部答复（§7），可以开工。行号基于 `708c1165a`，动手前先复核。

顺序：先做第一步（§3 到 §5）。第二步（§6）是按裁决整理过的思路，等第一步合入后再拆成任务。

## 1. 目标

owner 2026-11-27 对 Provider 实现的要求：

1. AICC 内部是一组协议客户端，每个客户端的 endpoint 由 Provider 的 base endpoint 推算。不随意写只给一家用的协议客户端，方便做协议兼容性 review。
2. 协议客户端是内核能力，决定能添加哪些自定义 Provider。
3. 模型列表的内容是“模型名对应协议客户端”，并允许把协议里的模型名映射到 AICC 标准模型名，映射可更新。
4. 价格可选，尽量通过协议拿实时价格。
5. 自定义 Provider：选协议族，填 endpoint 和 token，可选加一份本地配置。
6. **Adapter 清单表达的是我们对“现在世界上有哪些主流 AI API 类型”的理解。** 第一步要先 review 这份清单：哪些删，哪些加，哪些调整。
7. **只有两类调整需要发版本：API 类型升级、协议客户端升级。** 其余都应该能靠更新配置文件完成，包括内置 Provider 的能力升级。模型目录可以在线更新；目录不认识的模型名就是不识别。

第一步解决第 3、6 条：先定清单（§3），再让模型规则逐模型指向协议客户端，去掉组合类和多余的方言 Adapter。第二步落实第 5 条。第 4 条不在本文范围。第 7 条是贯穿两步的判断标准。

## 2. 现状

### 2.1 组合 Adapter 的成因

- 一个实例只有一个 `protocol_adapter_id`（`settings/mod.rs:47`、`provider/inventory.rs` 的 `ProviderInstanceConfig`）。
- 模型规则的 `operations` 是“API 类型或 method 对应 operation ID”（`catalog/schema.rs:628`），只能在这一个 Adapter 内解析。
- 所以一家用到多种 wire 协议时，只能注册一个专用 Adapter，把别的 Adapter 的 operation 和 codec 拷进自己名下，并用 `component_adapter_ids` 声明。
- `CodecRegistry` 的键已经是 `(adapter_id, operation_id, api_type)`（`protocol/adapter.rs:855`），所有查找都带 `adapter_id`。注册表本身不用改，要改的是 `adapter_id` 从哪里来。
- operation ID 只在一个 Adapter 内唯一。`audio.speech`、`images.generate`、`embeddings.create`、`chat.completions.create`、`videos.create` 都在多个 Adapter 里出现，跨客户端引用必须带上客户端 ID。

### 2.2 组合 Adapter 清单

| Adapter | 拼了什么 | 位置 |
| --- | --- | --- |
| `doubao-responses` | OpenAI Responses（只改了 provider state 命名空间）+ `doubao-media` + `doubao-speech` | `protocol/derived_responses.rs:103` |
| `glm-chat` | Chat Completions 的 GLM 方言 + `glm-media` 全部 operation | `protocol/chat_completions_dialects.rs:411` |
| `minimax-messages` | Claude Messages 的 MiniMax 方言 + `minimax-media` | `protocol/minimax_messages.rs:41` |
| `qwen-responses` | Responses 的 Qwen 方言 + 没有单独注册的 `qwen-aggregated-chat` + `qwen-media` | `protocol/derived_responses.rs:103`、`chat_completions_dialects.rs:92` |
| `openrouter-responses` | Responses 的 OpenRouter 方言 + OpenAI 的 embeddings 和 transcriptions + Chat 的 OpenRouter 方言（含 music）+ OpenRouter 自己的 speech、images、rerank、decisions、videos | `protocol/chat_completions_dialects.rs:134` |
| `sn-openai` | OpenAI Responses 全部 operation + `responses.background` + 用 Chat Completions 做的 ASR + 改过的 videos（不能取消、支持 extend） | `protocol/sn_openai.rs:17`、`:48` |

### 2.3 各家规则对客户端的需求（按现有 `.provider.json` 统计）

- GLM、MiniMax、豆包三个 profile、Qwen：每条模型规则的 operation 都落在同一个组件里。给规则指定一个客户端就够。
- OpenRouter：22 条带 operation 的规则里有 7 条跨组件（同一个模型，LLM 走 Responses，图片走 OpenRouter 原生接口，等等）。必须能按 API 类型指定客户端。
- SN：跨的主要是 `responses.create` 与 `responses.background`，这两个本来属于同一套 OpenAI Responses 协议；另有 3 条规则用 Chat Completions 做 ASR。

### 2.4 `protocol_adapter_id` 流经的位置

AICC 内部：

- 库存：`ProviderInventoryModel.operations`（`provider/inventory.rs:626`）、快照级 `protocol_adapter_id`、`filtered_capabilities`。
- operation 解析有三处，结果必须一致：`provider/mod.rs:613` `resolve_operation`（建库存）、`routing/mod.rs:1060` `resolve_operation`（选路）、`call/mod.rs:696` `resolve_operation` 与 `:783` `select_operation`（lowering，不一致时报 `RouteOperationMismatch`）。
- 模型注册表：`ModelUid`（`model/mod.rs:89`，格式 `driver:model:adapter[:variant]`）、`identity.protocol_adapter_id`、三处 `operations`（`model/mod.rs:166`、`:274`、`:294`）。
- 选路结果：`SelectedRoute.protocol_adapter_id`（`routing/mod.rs:124`）。
- lowering：`ResolvedProviderCall.protocol_adapter_id`（`call/mod.rs:93`）；`CodecContext.state_coordinate.adapter_type`（`call/mod.rs:647`）；base URL 取 `operation_base_urls[operation]`，没有就用实例 base（`call/mod.rs:630`）。
- 执行：`service/provider_execution.rs`（27 处引用）。
- 存储：`aicc_provider_inventory_lkgs.protocol_adapter_id`、`aicc_artifact_url_source.protocol_adapter_id`（`storage/schema.sql:5`、`:58`）。

AICC 外部：

- `kernel/buckyos-api/src/aicc_client.rs`（13 处，含 `:4515` `base_adapter_id`、`:4517` `component_adapter_ids`）；`buckyos-websdk/src/aicc_client.ts`；Desktop `src/app/ai-center/` 与 `src/api/aicc_mgr.ts`（13 个文件）；`tools/buckyos-agent/commands/ai_provider.ts`。
- `kernel/scheduler/src/system_config_builder.rs:123`、`:175`：托管 SN 实例写死 `"sn-openai"`，并据此判断要不要改写。
- `frame/agent_tool/src/llm_understand_media/pipeline.rs:328` `protocol_accepts_documents`：按 Adapter ID 白名单（`claude-messages`、`gemini-interactions`、`openai-responses`）判断能否发文档。
- `src/dev_configs/aicc.json`。

## 3. Adapter 清单 review

清单的处置 owner 已确认（Q2 到 Q9）。关于各家对外接口的判断来自写作时已有的了解，没有逐项对照厂商当前文档；实施每一项前按官方文档核对，沿用 metadata 里 `source_url` / `verified_at` 的做法。

### 3.1 判断方法

一个协议客户端实现一种 API 类型。对每个客户端都要回答两个问题：它是哪一种 API 类型；除了第一家，还有谁在用。按答案分三层：

- **A 跨厂商事实标准。** 多家厂商、聚合网关、本地推理都实现。做成标准客户端，对自定义 Provider 开放。
- **B 官方独有。** 只有一家在用，但它是这家的官方主接口。保留，不对自定义开放。
- **C 厂商原生。** 媒体、语音、异步任务这类没有跨厂商标准的接口。保留为单厂商客户端，不对自定义开放。

规则：

1. 标准客户端分两种口径，分开实现（Q3、N1）：
   - **官方口径**：按第一家当前的官方文档严格实现，ID 不变。
   - **生态口径**：按兼容厂商和本地推理的通行约定实现。是单独的客户端，ID 加 `legacy-` 前缀，`base_adapter_id` 指向官方口径的客户端，只覆盖差异。
2. 各家在生态约定之内的差别（带哪些扩展参数、去掉哪个字段），用规则里已有的 `provider_options`、`request_rules`、`variants` 表达，不进客户端代码。这些可以在线更新（目标第 7 条）。
3. 方言客户端只在上面的办法都表达不了时才留，并在清单里写明残余差异。
4. SN 基于 LiteLLM，LiteLLM 本身是主流网关。SN 用标准客户端接入，不留 `sn-openai`，也不为它新增客户端（Q5）。

### 3.2 现有 Adapter 的处置

已注册 19 个，另有 1 个没注册、藏在组合里的 `qwen-aggregated-chat`。

对话类：

| Adapter | 实际是什么 | 处置 | 说明 |
| --- | --- | --- | --- |
| `openai-chat-completions` | Chat Completions，按 OpenAI 官方严格实现：发 `max_completion_tokens`，不认 `reasoning_content`（`openai_chat_completions.rs:62` 起的默认方言） | 保留，官方口径 | 指向 OpenAI 本身或严格兼容端点时用。OpenRouter 的 Chat、SN 的 ASR 现在走的也是这个口径 |
| `kimi-chat` | 生态约定（`max_tokens`、`reasoning_content` 的解析与回放、`thinking` 类开关）+ `prompt_cache_key`、`partial` | 删除，并入 `legacy-openai-chat-completions` | 残余：`partial` 续写（`chat_completions_dialects.rs:479`） |
| `glm-chat` | 生态约定 + `tool_stream`、去掉 `stream_options`、响应缺 `object` 时补上 | 删除，并入 `legacy-openai-chat-completions` | 残余待核对：`tool_stream` 的流式分片归一化（`:539`） |
| `qwen-aggregated-chat` | 生态约定 + `enable_thinking`、`preserve_thinking`、`tool_stream` | 删除，并入 `legacy-openai-chat-completions` | 三个方言的 `transform_*` 大部分是同一段代码（`:602` 起） |
| `openai-responses` | 名字是 Responses，实际装了 OpenAI 整个平台：responses、images、audio、embeddings、videos | 调整，拆开 | 拆分粒度按“自定义 Provider 会怎么组合”判断（Q4）。`glm-media` 已经在原样复用其中四个 codec |
| `deepseek-responses` | Responses；差异只有 base 没带路径时的 endpoint 和命名空间 | 删除 | 由 §4.5 的命名空间和 P1-11 的 base 推算覆盖 |
| `doubao-responses` | Responses；只改命名空间 | 删除 | 同上 |
| `qwen-responses` | Responses + session cache 头、`enable_thinking`、视频输入从 `input_file` 改写成 `input_video` | 调整，只留方言 | 视频输入改写要写代码（`derived_responses.rs:342` 起） |
| `openrouter-responses` | Responses + OpenRouter 路由参数、`web_search` 工具改名、自报费用 | 调整，只留方言 | OpenRouter 是影响大的聚合商，留一个方言可以接受 |
| `sn-openai` | OpenAI 平台 + `responses.background` + Chat 做的 ASR + videos 变体 | 删除 | Q5。残余并入标准客户端，见 P1-7 |
| `claude-messages` | Anthropic Messages，按官方严格实现：`max_tokens` 必填，temperature 上限 1 | 保留，官方口径 | |
| `minimax-messages` | Messages + `max_tokens` 可省、temperature 到 2、成功响应里的 `base_resp` 错误 | 删除，并入 `legacy-claude-messages` | 残余待核对：`base_resp` 错误映射（`minimax_messages.rs:226` 起） |
| `gemini-interactions` | Interactions + `models.predictLongRunning` + `models.embedContent` | 调整，不再对自定义开放 | Interactions 只有 Google 自己在用，归 B 层（Q6）。后两个属于 `models.*` 方法族，移到新增的 `gemini-generate-content` |

媒体与其它：

| Adapter | 实际是什么 | 处置 | 说明 |
| --- | --- | --- | --- |
| `glm-media` | embeddings、images、speech、transcriptions 四个直接复用 OpenAI 的 codec（`glm_media.rs:31`）；只有视频（`videos/generations` 加 `async-result`）是 GLM 自己的 | 调整，缩成 `glm-video` | 前四个改为引用拆出来的 OpenAI 平台客户端 |
| `minimax-media` | MiniMax 原生：T2A v2、图片、音乐、视频任务、语音识别 | 保留（C） | |
| `doubao-media` | 方舟原生：图片、内容生成任务（视频）、多模态向量 | 保留（C） | 图片的路径与 OpenAI 相同、codec 是自己的，是否能算 OpenAI Images 的方言待核对 |
| `doubao-speech` | 火山 openspeech 原生：流式 TTS、大模型 ASR | 保留（C） | |
| `qwen-media` | DashScope 原生异步任务：图片、视频 | 保留（C） | |
| `fal-queue` | fal 队列 API | 保留（C） | |
| `typesafe-systemone` | TypeSafe Decision | 保留（C），取消对自定义开放（Q7） | workflow 引擎在用 `decision` |

原生客户端不改 ID，平台名（方舟、DashScope 等）只写在客户端清单里（Q8）。

藏在 `openrouter-responses` 里的部件：

| 部件 | 实际是什么 | 处置 |
| --- | --- | --- |
| Chat 方言（`chat_completions_dialects.rs:24`） | 官方口径的 Chat Completions + 只允许 text modality；music 也走这个接口 | 改用 `openai-chat-completions`，残余归 `openrouter-native` 或规则 |
| `rerank.create`（`:246`） | Cohere / Jina 风格的 `/rerank`，多家厂商和自建推理都实现 | 上提为标准客户端 `rerank`（A） |
| embeddings、transcriptions | 原样复用 OpenAI 的 codec | 改为引用 OpenAI 平台客户端 |
| speech、images、videos、decisions | OpenRouter 自己的 codec | 归 `openrouter-native`（C）。videos 的路径形状与 OpenAI Videos 相近，是否算方言待核对 |

### 3.3 新增

- **`legacy-openai-chat-completions`**：生态口径的 Chat Completions。范围见 §4.7。
- **`legacy-claude-messages`**：生态口径的 Messages。范围见 §4.7。
- **`gemini-generate-content`**：Gemini 的 `models.*` 方法族（`generateContent`、`streamGenerateContent`、`embedContent`、`predictLongRunning`）。市面上的 “Gemini 兼容” 网关实现的都是它，不是 Interactions；现在自定义 Provider 选 “Gemini compatible” 实际拿到的是 Interactions。Gemini 的模型发现本来就按 `supportedGenerationMethods: ["generateContent"]` 判断（`provider/builtin/gemini.rs`）。owner 提到 Gemini 最近发了新版本，实施前要先做一轮 review（P1-12）。
- **`openai-embeddings`、`openai-images`、`openai-audio`、`openai-videos`**：从 `openai-responses` 拆出（Q4）。
- **`rerank`**：从 OpenRouter 里上提。

### 3.4 目标清单

| 层 | 客户端 | 对自定义开放 |
| --- | --- | --- |
| A 标准，官方口径 | `openai-responses`、`openai-chat-completions`、`claude-messages`、`gemini-generate-content`、`openai-embeddings`、`openai-images`、`openai-audio`、`openai-videos`、`rerank` | 是 |
| A 标准，生态口径 | `legacy-openai-chat-completions`、`legacy-claude-messages` | 是 |
| B 官方独有 | `gemini-interactions` | 否 |
| 方言 | `qwen-responses`、`openrouter-responses` | 否 |
| C 原生 | `fal-queue`、`minimax-media`、`doubao-media`、`doubao-speech`、`qwen-media`、`glm-video`、`openrouter-native`、`typesafe-systemone` | 否 |

合计 22 个，现在是 19 个。构成的变化：

- 标准客户端从 4 个变成 11 个（官方口径 9、生态口径 2），另有 1 个官方独有。
- 方言和组合从 8 个（`kimi-chat`、`glm-chat`、`minimax-messages`、`deepseek-responses`、`doubao-responses`、`qwen-responses`、`openrouter-responses`、`sn-openai`）变成 2 个。前提是 Kimi、GLM、MiniMax、SN 的残余能并入标准客户端或用配置表达；并不进去的逐项报告 owner，不自行保留。
- 原生从 7 个变成 8 个：多了 `openrouter-native`，`glm-media` 缩成 `glm-video`。

这份清单就是“改它需要发版本”的全部内容（目标第 7 条）。

### 3.5 登记但不做的候选

第一步不做（Q11）：

- 云平台托管，美欧日企业常用：
  - [ ] Azure OpenAI：先核实。预计只需要一个 Known Provider，用标准 OpenAI 客户端加 `api-key` 头，不需要新客户端。
  - AWS Bedrock：Converse API，SigV4 签名或平台 API key。是新的 API 类型，也是新的认证方式。有客户需求再排。
  - Google Vertex AI：wire 与 Gemini `models.*`、Anthropic Messages 基本一致，差在 OAuth 令牌和路径。主要是新的认证方式。有客户需求再排。
- 语音专用：ElevenLabs（TTS）、Deepgram 和 AssemblyAI（ASR）。
- 实时会话（WebSocket / WebRTC，例如 OpenAI Realtime、Gemini Live）：AICC 目前没有这类 API 类型。
- Replicate 预测 API：与 fal 同类的聚合平台。

不需要单独支持：Ollama、LM Studio、vLLM、llama.cpp 的原生接口，它们都提供 Chat Completions 兼容端点。

### 3.6 不符合目标第 7 条的存量

下面这些是写在代码里的 Provider 事实，改它们现在要发版本。第一步只处理标了任务号的，其余登记：

- 模型发现（P1-13）：注册了 14 个 `discovery_behavior_id`（`provider/builtin/registry.rs:36`），实际只有四类：
  - 协议族的标准列表，各有一份共用实现。OpenAI `/models`：`openai-models`、`kimi-models`、`glm-models`、`deepseek-models`、`openai-compatible-models`、`standard-models`，都调 `openai_responses_compatible.rs:563` 的 `discover_model_ids`。Anthropic `/v1/models`：`anthropic-models`、`minimax-models`，后者只差不发版本头。Gemini `models`：`gemini-models`。
  - 标准列表加这家自己的字段：`volcengine-ark-models` 多读 `status`、`task_type`；Kimi 的三个字段写成了共用函数里的 `if provider == "kimi"`（`:619`）。
  - 这家自己的目录协议：`openrouter-models`（模态、支持的参数、价格，另有 `videos/models`）；`sn-models`（自己的信封，只取模型 ID）。
  - 不联网：`catalog-only`、`typesafe-catalog`，列表来自规则文件。
  每个 ID 的包装代码主要在校验“profile 和 Adapter 是不是自己”，其中写死了 `kimi-chat`、`sn-openai` 等将被删除的 ID（P1-2）。
- 写在发现代码里的模型事实：OpenRouter 的厂商前缀映射（`openrouter.rs:111`，P1-13）和 Jev 版本常量（P1-13 只核对并报告）；TypeSafe 把 `jev-latest`、`jev-preview` 指向 `jev-1.13.0`（`typesafe.rs:13`，P1-13）；fal 的价格查询靠 `if provider_profile_id == "fal"` 挂上去（`registry.rs:329`，登记）。
- `call/mod.rs:634`：按 `provider_profile_id == "doubao"` 且 URL 含 `/plan/` 清空价格。
- `provider/builtin/fal.rs:75`：写死的价格查询地址（P1-11）。
- `agent_tool` 的 `protocol_accepts_documents`：按 Adapter ID 白名单判断能力（P1-7）。
- 各客户端里按 Provider 的 base 路径做的改写（P1-11）。
- Desktop 里按 profile ID 写的分支（`src/api/aicc_mgr.ts` 的 `isDoubaoProfile` 等）。

## 4. 第一步的目标设计

### 4.1 协议客户端

一个 `protocol_adapter_id` 对应一种 API 类型：一组 endpoint 路径、请求和响应结构、凭据放在哪个头。客户端只注册自己实现的 codec，不再把别的客户端的 operation 拷进来。`component_adapter_ids` 删除。

生态口径的客户端和留下来的方言客户端：`base_adapter_id` 指向官方口径的客户端，只覆盖差异，其余继承，与基础客户端同属一个 `protocol_family_id`。

协议族是一组可以共用同一个 endpoint 和凭据的标准客户端：

- OpenAI 族：`openai-responses`、`openai-chat-completions`、`legacy-openai-chat-completions`、`openai-embeddings`、`openai-images`、`openai-audio`、`openai-videos`。
- Claude 族：`claude-messages`、`legacy-claude-messages`。
- Gemini 族：`gemini-generate-content`、`gemini-interactions`。
- `rerank` 自成一族。

### 4.2 实例的客户端集合

- Known Provider：`protocol_adapter_id` 保留，含义改为“默认客户端”；新增有序列表 `extra_protocol_adapters`，每项至少有 `id`，可选 `credential`（见 §4.4）。实例的客户端集合只从 catalog 来，settings 里的 `protocol_adapter_id` 对 Known Provider 不再读取。
- 自定义实例：settings 里的 `protocol_adapter_id` 是默认客户端；客户端集合是它所在协议族里对自定义开放的全部客户端。
- 规则里引用的客户端必须在实例的集合内，否则 catalog 校验失败。
- 两份声明都在可在线更新的配置里。让一家 Provider 多用一个已有的客户端，不需要发版本。

### 4.3 模型规则指向客户端

一个绑定是“客户端 + operation”（Q1）。有映射时的解析顺序：

1. `operations` 的值写成 `<protocol_adapter_id>/<operation_id>` 时，用它指定的客户端。`/` 不在两种 ID 的合法字符里（operation ID 只允许字母数字和 `-_.`，见 `protocol/adapter.rs:1525`）。
2. 否则用规则级的 `protocol_adapter_id`（`models[]` / `patterns[]` 的新可选字段）。
3. 否则用实例的默认客户端。

某个 API 类型没有任何映射命中时：

1. 先在第 2、3 步确定的那个客户端里按 `default_priority` 选。
2. 这个客户端没有该 API 类型的绑定时，在集合里其余客户端中选 `default_priority` 最高的唯一一个。
3. 并列报歧义，不猜。`default_priority` 为 0 的绑定只能显式选中。

示例：

```json
{ "id": "cogview-4", "protocol_adapter_id": "openai-images",
  "operations": { "image.txt2img": "images.generate" } }
```

```json
{ "id": "x-ai/grok-4.7",
  "operations": {
    "llm": "responses.create",
    "image.txt2img": "openrouter-native/images.generate",
    "embedding.text": "openai-embeddings/embeddings.create"
  } }
```

库存、模型注册表、选路结果里的 `operations` 一律存限定后的完整形式，不再依赖“实例的 Adapter”这个隐含上下文。

### 4.4 凭据按客户端呈现

实例只有一份密钥。发请求时按选中的客户端决定怎么放进请求头，优先级：

1. 实例显式选的 `credential_kind`（例如 GLM 的 `glm_jwt`）；
2. Known Provider 给这个客户端写的 `credential`；
3. 客户端描述里的 `AdapterDescriptor.credential`。

`minimax_media.rs:971` `apply_media_credential`（把 `x-api-key` 的密钥取出来重新拼成 Bearer）随之删除。

### 4.5 Provider state 命名空间由上下文给

现在 `doubao-responses` 相对 OpenAI Responses 唯一的差异，是把输出里 `provider_state.provider` 从 `openai` 改写成 `doubao`（`derived_responses.rs:596` 起的 `rewrite_*_namespace`；MiniMax 在 `minimax_messages.rs:291`）。Chat 方言里也按 `provider == "kimi"` 这样的名字识别自己的状态。

改成：Known Provider 可选声明 `state_namespace`，经 `CodecContext` 传给客户端，客户端直接按它读写。不声明时用客户端自己的命名空间。现有各家的命名空间取值（`doubao`、`qwen`、`openrouter`、`deepseek`、`minimax`、`kimi`、`glm`）保持不变。

### 4.6 各 Provider 用到的客户端

| Provider | 默认客户端 | extra 客户端 |
| --- | --- | --- |
| openai | `openai-responses` | `openai-embeddings`、`openai-images`、`openai-audio`、`openai-videos` |
| claude | `claude-messages` | 无 |
| gemini | `gemini-interactions` | `gemini-generate-content`（承接 embedContent、predictLongRunning） |
| deepseek | `openai-responses` | 无 |
| kimi | `legacy-openai-chat-completions` | 无 |
| glm | `legacy-openai-chat-completions` | `openai-embeddings`、`openai-images`、`openai-audio`、`glm-video` |
| minimax | `legacy-claude-messages` | `minimax-media` |
| doubao、doubao-agent-plan | `openai-responses` | `doubao-media`、`doubao-speech` |
| doubao-speech | `doubao-speech` | 无 |
| qwen | `qwen-responses` | `legacy-openai-chat-completions`、`qwen-media` |
| openrouter | `openrouter-responses` | `openai-chat-completions`、`openai-embeddings`、`openai-audio`、`rerank`、`openrouter-native` |
| sn | `openai-responses` | `openai-chat-completions`、`openai-embeddings`、`openai-images`、`openai-audio`、`openai-videos` |
| fal | `fal-queue` | 无 |
| typesafe | `typesafe-systemone` | 无 |

### 4.7 生态口径客户端的范围

只收多家都这么做的约定。只有一家这么做的不进来。

`legacy-openai-chat-completions`（基础是 `openai-chat-completions`）：

- 输出上限参数发 `max_tokens`。
- 历史里的 thinking 文本作为 `reasoning_content` 回放给上游。
- 响应和流里的 `reasoning_content` 或 `reasoning` 解析成 thinking。
- 顶层的 `cached_tokens` 归到 `prompt_tokens_details`；响应缺 `object` 不报错。
- 接受一组生态里通行的扩展参数并做类型校验：`thinking`、`enable_thinking`、`preserve_thinking`、`tool_stream`、`prompt_cache_key`。哪家实际带哪些，由规则决定。

`legacy-claude-messages`（基础是 `claude-messages`）：

- `max_tokens` 可以不发。
- temperature 上限放到 2。

明确不进来的，按 §3.1 规则 3 处理：Kimi 的 `partial` 续写、GLM 的 `tool_stream` 分片归一化、MiniMax 的 `base_resp` 错误映射。

## 5. 第一步任务

硬约束：

- **内置 Provider 对上游发出的请求不变。** 第一步是内部结构调整，任何 URL、头、请求体的变化都算缺陷。Kimi、GLM、MiniMax 等并入生态口径客户端后，发出的字节必须和现在一样。
- 不新增运行时探测或失败回退。
- 规则数据只改 `driver_metadata/*.json`。Rust builtin 模块不构造生产用 catalog（`provider_profile_schema.md` §12 第 11 条）。
- 能用配置表达的差异不写进客户端代码（目标第 7 条）。
- 这个版本是 breaking change（Q10，仓库 `AGENTS.md` 的默认也是如此）：不迁移旧数据，不保留旧格式。已知后果列在验收之后。
- 每个任务单独可合入。迁移期间旧 Adapter 和新写法并存，P1-8 才删旧机制。
- §3 里标了“待核对”的处置，核对结果与预期不符时先报告 owner。

### 地基

**P1-0 基线与护栏**

- [ ] 在 `src/` 下跑 `cargo test -p aicc -- --test-threads=1`，记下基线失败的用例。
- [ ] 加一份 wire 快照测试：对 `doc/aicc/provider_operation_bindings.md` 里每一行绑定，用该 API 类型的最小请求做一次 lowering 加 encode，记录 method、URL、头的名字、请求体。请求构造可以复用 `call/mod.rs` 测试里 `custom_provider_lowering_has_a_default_for_every_selectable_binding`（`:2626`）一带已有的各类型请求。NativeTask 绑定记录 submit 请求。
- [ ] 快照以“Provider + 模型样例 + API 类型”为键，不含 Adapter ID，这样迁移前后可以逐字节比较。
- [ ] 方言会被并入生态口径客户端，所以快照还要覆盖方言特有的行为：带 thinking 历史的多轮请求、各家的扩展参数、流式响应的解码结果（thinking 增量、用量）。解码结果也记进快照。

**P1-1 绑定引用**

- [ ] 新增类型表示“客户端 + operation”，带 `<adapter>/<operation>` 的解析和输出。
- [ ] `ProviderExactRule` / `ProviderPatternRule`（`catalog/schema.rs` 的 `define_provider_rule!`）加可选 `protocol_adapter_id`；`operations` 的值接受限定写法。
- [ ] 这两项是新 schema 能力，按现有做法用 `schema_revision` 把关（参考 `catalog/validation.rs:304` 对 `custom_provider_adapters` 的处理）。
- [ ] 把三处 operation 解析合成一个函数，输入“客户端集合、规则、discovery 的 `remote_methods`、API 类型、method”，按 §4.3 输出限定后的绑定。库存、选路、lowering 都调它。`RouteOperationMismatch` 检查保留。
- [ ] `ProviderInventoryModel.operations`、模型注册表、`SelectedRoute` 存限定形式。`SelectedRoute.protocol_adapter_id` 和 `ResolvedProviderCall.protocol_adapter_id` 改为本次请求实际选中的客户端。
- [ ] 库存快照的 `schema_version` 加一，旧快照不再加载。
- [ ] `ModelUid` 的 adapter 分量填模型的默认客户端（§4.3 有映射时第 2、3 步的结果）。它只用于展示和 `selected_model_uid`，不参与查找。
- [ ] 这一步结束时所有内置规则仍用旧写法，行为和快照不变。

**P1-2 实例的客户端集合**

- [ ] `KnownProvider`（`catalog/schema.rs:791`）加 `extra_protocol_adapters`、`state_namespace`；`ResolvedProviderConfiguration`、`ProviderProfile`（`provider/mod.rs:94`）带上客户端集合。
- [ ] 校验：extra 里的 ID 已注册、不重复、不等于默认；规则引用的客户端在集合内；每个限定绑定在注册表里存在。校验放在能拿到 `CodecRegistry` 的地方（`provider/builtin/registry.rs` 的 `BuiltinProviderRegistry::new` 一带）。
- [ ] Known Provider 实例不再读取 settings 里的 `protocol_adapter_id`，去掉对应的一致性检查（`provider/runtime.rs:379`、`:611`）。管理接口不再要求 Known Provider 提交这个字段。
- [ ] 自定义实例的客户端集合按 §4.2 取协议族。
- [ ] discovery 行为构造时传入的 `protocol_adapter_id`（`registry.rs` 的 `BuiltinDiscoveryFactory` 分支）改为默认客户端。各发现包装里校验 Adapter ID 的代码写死了旧 ID（`kimi.rs` 的 `validate_context`、`sn.rs` 的 `validate_sn_context`、`openai.rs` 的 `validate_openai_context` 等），改成只校验 profile。

**P1-3 凭据与命名空间**

- [ ] 凭据按 §4.4 呈现。现在 `ProviderCallTarget.credential`（`call/mod.rs:43`）在选路前就已经是按 profile 渲染好的 `ResolvedCredential`，需要改成选定绑定后再渲染，或者让它带着密钥按客户端渲染。
- [ ] `CodecContext` 加命名空间，标准客户端读写 provider state 时用它。
- [ ] 这一步结束时旧 Adapter 都还在，快照不变。

### 标准客户端

**P1-4 拆 OpenAI 平台**

- [ ] 从 `openai-responses` 拆出 `openai-embeddings`、`openai-images`、`openai-audio`、`openai-videos`，各自独立注册。`openai-responses` 只留 `responses.create` 和 `responses.background`。
- [ ] `responses.background`（`openai_responses.rs:2813`）注册成 `openai-responses` 的正式 operation（Q9），`default_priority` 为 0，只能显式选中。确认它不改变默认选择（`provider/tests.rs:2025` 一组用例）。
- [ ] `openai.known-provider.json` 加四个 extra 客户端，`openai.provider.json` 的规则加限定。
- [ ] 四个新客户端对自定义开放。自定义实例靠 §4.2 的协议族规则拿到它们，用 `provider/tests.rs` 里自定义实例的那组用例确认图片、语音、向量的默认 operation 仍然唯一。

**P1-5 生态口径客户端**

- [ ] 新增 `legacy-openai-chat-completions`：把 `KimiDialect`、`GlmDialect`、`QwenAggregatedDialect` 共有的部分收成一个实现，范围按 §4.7。
- [ ] Kimi、GLM、Qwen 聚合模型改用它。各家带哪些扩展参数、GLM 去掉 `stream_options` 这类差别，移到各家规则的 `provider_options`、`request_rules`、`variants`。
- [ ] 逐项对照三个方言的六个钩子（`openai_chat_completions.rs:62`），列出并入后还剩什么。预计是 Kimi 的 `partial`、GLM 的 `tool_stream` 分片归一化。
- [ ] 新增 `legacy-claude-messages`，MiniMax 改用它。预计残余是 `base_resp` 错误映射。
- [ ] OpenRouter 的 Chat 改用 `openai-chat-completions`。预计残余是 text modality 约束和给图片输入追加的那句指令。
- [ ] 残余逐项报告 owner：能用配置表达的用配置；不能的由 owner 决定是放弃这项行为，还是留一个只含这一项的方言客户端。不要为了删 ID 把单家的做法塞进标准客户端。
- [ ] 删除 `kimi-chat`、`qwen-aggregated-chat`；`glm-chat`、`minimax-messages` 在残余有结论后删除。

**P1-6 rerank 上提**

- [ ] `OpenRouterRerankCodec` 改成标准客户端 `rerank`。它现在自己把 base 补成 `/api/v1`（`chat_completions_dialects.rs:300` 一带），这段路径处理改由 P1-11 的 base 推算表达；P1-11 之前先保持 URL 不变。

### 逐家迁移

**P1-7 逐家迁移并删除旧 Adapter**

顺序：豆包，DeepSeek，Kimi，GLM，MiniMax，Qwen，OpenRouter，SN。每家做完都要过快照，并更新 `provider_operation_bindings.md`（由 `call::tests::every_builtin_provider_operation_has_a_golden_lowering_binding` 校验，`call/mod.rs:3008`；该测试里 Adapter 列改为实际客户端）。

每家的固定动作：

- [ ] `known-providers/<厂商>.known-provider.json` 按 §4.6 改默认客户端、加 `extra_protocol_adapters`、`state_namespace`；`providers/<厂商>.provider.json` 给走非默认客户端的规则加 `protocol_adapter_id` 或限定写法。
- [ ] 删除旧 Adapter 的注册和拷贝组件的代码。
- [ ] 改掉断言组合关系的测试：`derived_responses.rs:708` `derived_adapters_declare_base_and_provider_media_components`、`provider/builtin/minimax.rs:247`、`provider/builtin/upgrade_tests.rs` 里按 Adapter ID 断言的用例。

各家的特别事项：

- **豆包**：删除 `doubao-responses` 和 `ResponsesDialectKind::Doubao`。三个 profile 一起改。
  - `call/mod.rs:634` 的价格特判保持行为，不在这一步清理（§3.6）。
  - `protocol_accepts_documents`（agent_tool）对 `openai-responses` 返回 true，迁移后豆包的 LLM 路由会从“不发文档”变成“发文档”。把这个判断改成看模型能力（Model Driver 里的事实），不再看 Adapter ID；改之前确认方舟 Responses 是否接受 `input_file`。DeepSeek、SN 同理。
- **DeepSeek**：`deepseek-responses` 的 endpoint 差异（base 没带路径时用 `/responses`）在 P1-11 之前先由 Known Provider 的 base 写全来保持 URL 不变。非 OpenRouter 方言去掉 `web_search` 能力的做法（`derived_responses.rs:116`）改成规则里的 `remove_features`。
- **GLM**：`glm-chat` 描述里的凭据是 `GlmJwt`（`chat_completions_dialects.rs:459`）。确认迁移后 JWT 和 Bearer 两种实例下，各客户端的头都和现在一致。`glm-media` 缩成 `glm-video`。
- **MiniMax**：`minimax-media` 的凭据在 Known Provider 里写 Bearer，删 `apply_media_credential`。
- **Qwen**：`qwen-responses` 只留 Responses 方言。聚合模型的 Chat 改用 `legacy-openai-chat-completions`。
- **OpenRouter**：新增 `openrouter-native`。`audio.speech`、`images.generate` 在 OpenRouter 用的是自己的 codec，和 OpenAI 的同名 operation 不是一回事。自报费用币种现在挂在 Responses 方言上（`derived_responses.rs:106`），确认迁到别的客户端的 operation 仍按 `openrouter.provider.json` 的 `reported_cost` 计费。
- **SN**（Q5：用标准客户端，删除 `sn-openai`）：
  - 默认客户端改 `openai-responses`，extra 是 OpenAI 族的标准客户端。动态登录是 Provider 行为，不属于协议客户端，保持不变。
  - Chat 做的 ASR（`sn_openai.rs:248`）：做成 `openai-chat-completions` 的 `audio.asr` 绑定，`default_priority` 为 0，SN 规则显式引用。
  - videos 的两处差异（支持 extend、不能取消）：先核对 OpenAI Videos 和 LiteLLM 的现状。属于标准的并入 `openai-videos`，extend 作为显式绑定；“不能取消”改成规则里声明的能力。并不进去的报告 owner。
  - scheduler 同步：`kernel/scheduler/src/system_config_builder.rs:123`、`:175` 不再写、也不再按 `"sn-openai"` 判断；`kernel/scheduler/tests/sn_ai_routing.rs`、`tools/buckyos-agent/commands/ai_provider_test.ts` 里按这个 ID 断言的用例一并改。
  - SN 是多数 Zone 的默认 Provider，这一家必须做真实环境验证。

### 收口

**P1-8 注册表与清单**

- [ ] 删除 `AdapterDescriptor.component_adapter_ids` 以及 `validate_adapter_identity`（`protocol/adapter.rs:1054`）里“有组件就允许跨协议族”的分支。派生客户端必须与基础客户端同族。
- [ ] `typesafe-systemone`、`gemini-interactions` 取消对自定义的开放（Q6、Q7）。
- [ ] 新增一份由测试校验的客户端清单，放在 `doc/aicc/`：ID、所属层和口径、协议族、基础客户端、所属平台、operation 列表、方言的残余差异。新增或修改客户端必须同时改这份清单。`build.rs` 对 `protocol/plugins/` 的自动注册保留，清单是 review 的入口。
- [ ] 清单与 §3.4 一致；有出入的在实施记录里写明原因。

**P1-9 对外接口与界面**

- [ ] `buckyos-api`：去掉 `component_adapter_ids`；Provider catalog 视图加 `extra_protocol_adapters`；`FilteredProviderCapability.protocol_adapter_id` 填实际客户端。
- [ ] `buckyos-websdk` 同步类型，先发 websdk 再改 Desktop。
- [ ] Desktop：Provider 详情显示客户端集合；模型的 operation 显示成“客户端 · operation”；自定义 Provider 规则编辑器（`ProviderRulesPanel.tsx`、`src/api/custom_provider_rules.ts`）读写新写法时不丢字段。
- [ ] `tools/buckyos-agent/commands/ai_provider.ts` 的输出；按 `AGENTS.md` 的 cmd_skills 联动表检查 `buckyos_admin` 手册里涉及 Provider 的部分。
- [ ] 云端目录直接发新格式，不保留旧格式的发布轨道（Q10）。

**P1-10 文档**

- [ ] `provider_profile_schema.md` §2.1：去掉复合 Adapter 的描述和表格，改写成 §3、§4 的模型。把目标第 7 条写成规范。
- [ ] `driver_metadata_schema.md`、`provider_ui_backend_mapping.md`、`aicc-mgr.md`、`internal_module_architecture.md`、`aicc_provider_plan.md` 同步。
- [ ] 顺手改掉已有的矛盾：代码要求自定义 Provider 显式选客户端、不探测（`service/settings_runtime.rs:383`），但 `provider_profile_schema.md` §12 第 2、9 条、`provider_ui_backend_mapping.md`、`aicc-mgr.md` §4.3 仍写“后端自动探测”。以代码为准。

**P1-11 endpoint 推算收口（可延后，单独合入）**

目标是第 1 条：每个客户端的 endpoint 由 Provider 的 base 推出来，推不出来的在 metadata 里显式写明。

- [ ] `extra_protocol_adapters` 每项加 `base_url` 模板：`{base}` 是实例 base，`{origin}` 是它的协议加主机；另一个主机的情况写绝对地址。
- [ ] 用它替换：`operation_base_urls`（Known Provider 与实例两处）、`minimax_media.rs:871` 和 `:892` 丢掉 base 路径的 `set_path`、`qwen_media.rs:725` 剥 `/compatible-mode/v1` 的写法、豆包语音逐 operation 的地址、rerank 自己补 `/api/v1`、DeepSeek 的 `/responses`。
- [ ] 实例可以按客户端覆盖 base。向导里原来的逐 operation 地址输入改成逐客户端。
- [ ] `provider/builtin/fal.rs:75` 写死的价格查询地址改为从 metadata 来。
- [ ] 这一步会改 URL 的来源，但最终 URL 不变，仍用 P1-0 的快照验收。

**P1-13 模型发现归协议族（P1-2 之后随时可做；先做能省掉 P1-7 里各家包装的改动）**

裁决见 §7 的 N2。

- [ ] P1-0 的快照补上各内置 Provider 的发现请求（URL、头的名字），用它确认下面的整理没有改动请求。
- [ ] 模型列表归协议族：OpenAI `/models`、Anthropic `/v1/models`、Gemini `models` 各留一份实现，按实例默认客户端的协议族选。`BuiltinDiscoveryFactory::Standard` 已经是这个做法（`registry.rs:361`）。口径上的差别跟着默认客户端走：官方口径的 Claude 列表发版本头，生态口径不发（MiniMax 现在就不发）。
- [ ] 删除各家的包装和 ID：`openai-models`、`kimi-models`、`glm-models`、`deepseek-models`、`openai-compatible-models`、`anthropic-models`、`minimax-models`、`gemini-models`，Known Provider 统一写 `standard-models`。各家对 base 路径的处理（Kimi 的 `/v1`、DeepSeek 的根路径）在 P1-11 之前保持 URL 不变。
- [ ] 去掉列表里的模型事实（N2 第 1 条）：Kimi 的 `supports_reasoning`、`supports_image_in`、`supports_video_in`（`openai_responses_compatible.rs:619`）和方舟的 `task_type`。这是有意的行为变化，不在“请求不变”的约束内：去掉之前，用真实的列表响应对比前后的库存，差异逐条核对，目录缺的事实补进 Model Driver 或 Provider Rules。
- [ ] 方舟的 `status` 保留（下线中的模型标成不可用）：它是渠道当前的可用状态，不是模型事实。`volcengine-ark-models` 只剩“标准列表加 `status`”。
- [ ] SN 改用标准的 `/models`（N2 第 2 条）：先用真实 SN 确认 `{base}/models` 返回什么。现在的解析器要的是 `models` 或 `items` 信封（`sn.rs` 的 `parse_sn_models`），不是 OpenAI 的 `data` 列表。SN 是公共服务，旧版本 AICC 还在按旧信封解析；服务端怎么同时满足两边，核对后报告 owner。确认之前 `sn-models` 保留。
- [ ] TypeSafe 的别名（N2 第 3 条）：`jev-latest`、`jev-preview` 写进 `typesafe.provider.json` 的 `model_driver_overrides`，删除 `typesafe.rs` 的 `match_model_driver`，`typesafe-catalog` 并入 `catalog-only`。
- [ ] OpenRouter 的厂商前缀映射（N2 第 4 条）：`openrouter.provider.json` 加一个声明，把厂商前缀对到 Model Driver ID（`anthropic` 对 `claude`、`google` 对 `gemini`、`moonshotai` 对 `kimi`、`z-ai` 对 `glm`）；通用匹配链用它拆 `厂商/模型` 形式的 ID。删除 `openrouter.rs:97` `match_model_driver` 里对应的部分。字段的校验参照 `model_driver_overrides`。
- [ ] OpenRouter 发现代码里的 Jev 常量（渠道 ID、build ID、上下文 32000、模态，`openrouter.rs:163` 起）不在裁决范围内：核对能否用规则表达，结论报告 owner，不自行改。
- [ ] 做完后 `discovery_behavior_id` 只剩 `standard-models`、`catalog-only`、`openrouter-models`、`volcengine-ark-models`，以及 SN 确认前的 `sn-models`。把它们和各自的字段写进 P1-8 的客户端清单。

### 新增

**P1-12 `gemini-generate-content`（不依赖 P1-4 到 P1-11，可并行）**

- [ ] 先做 review（Q6）：按 Google 当前的官方文档确认 `models.*` 与 Interactions 各自的现状、默认版本路径、最近一次版本更新改了什么，以及兼容网关实现的是哪一套。结论写进实施记录；与 §3.3 的判断不一致时先报告 owner。
- [ ] 实现 `models.*` 方法族：`generateContent`、`streamGenerateContent`，并把 `models.embedContent`、`models.predictLongRunning` 从 `gemini-interactions` 移过来。
- [ ] Gemini 官方 profile 的 LLM 仍走 Interactions，行为不变；向量和长任务改引用新客户端，URL 不变。
- [ ] 新客户端对自定义开放。

### 第一步验收

- [ ] P1-0 的 wire 快照在每个任务后都不变。
- [ ] `cargo test -p aicc -- --test-threads=1` 相对基线没有新失败；`test/aicc_test/acceptance` 的 T1 用例通过。
- [ ] `protocol_adapter.list` 里没有 `component_adapter_ids`；客户端清单与 §3.4 一致，或差异有记录。
- [ ] 配置可更新（目标第 7 条）：用测试证明下面两件事只改 `driver_metadata`（等同云端下发）就生效，不改 Rust：给某家加一个走已有客户端的新模型；让某家多用一个已有的客户端。
- [ ] 真实调用：本机 Zone 上的托管 SN 至少验证 LLM、图片、ASR、视频各一次；其余迁移过的厂商有 key 的各验证一次 LLM（带 thinking 的多轮）和一次媒体调用。没有 key 的在实施记录里写明未验。

已知并接受的后果（不做兼容）：

- 会话里已有的 thinking 状态，来源坐标的 `adapter_type` 是旧 Adapter ID，升级后被当成外来状态丢弃一次（`provider_state.rs` 的 `drop_foreign_thinking`）。
- 库存快照重建一次。
- `aicc_artifact_url_source` 里记着已删除 Adapter ID 的旧行不迁移，这些旧产物地址不能再经 AICC 读取（`open_artifact_url_reader`，`protocol/adapter.rs:1035`）。
- 用 `gemini-interactions`、`typesafe-systemone` 建的自定义实例不再有效，需要重建。
- 旧版本 AICC 读不了新格式的云端目录。

## 6. 第二步思路：只填协议、endpoint、token 的自定义 Provider 基本可用

“基本可用”的定义（Q16）：

- 加了这个自定义 Provider 后，能对上模型目录的模型不用再写任何规则，就能进入用途并完成对话。
- 对不上模型目录的模型名保持不识别、不可调用。出路是更新模型目录（在线下发或本地覆盖），或者在实例规则里把它映射到已有模型。
- 费用未知时显示未知，不显示 0。

思路：

**S1 向导选协议族和默认的对话客户端。** 现在自定义能选哪些客户端由各家 `.provider.json` 的 `custom_provider_adapters` 决定，改成客户端自己声明，规则文件里的白名单删除。向导选协议族（OpenAI、Claude、Gemini），实例按 §4.2 带上族里对自定义开放的全部客户端。族里有多个对话客户端时再选一次默认：OpenAI 族默认 `legacy-openai-chat-completions`（Q12，填自定义端点的多数是第三方或本地推理），可改成 `openai-chat-completions` 或 `openai-responses`；Claude 族默认 `claude-messages`；Gemini 族只有 `gemini-generate-content`。

**S2 个别模型可以指到别的客户端。** 实例规则用第一步的规则级 `protocol_adapter_id`，界面上是模型旁边的一个下拉框。同一个网关同时提供多种接口时，不再需要建多个实例。跨协议族时依赖 P1-11：各族的标准客户端对 base 路径的默认处理不同（OpenAI 补 `/v1`，Gemini 补 `/v1beta`）。

**S3 认识的模型自动进入用途（Q13）。** 现在自定义实例没有思考档位（`provider/inventory.rs:1155`），基础模型只有写了 `server_default_routes` 才进 `llm.chat`（`model/mod.rs:965`）。改成：自定义 Provider 的模型只要对上了模型目录，就按已有的“端点默认”方式（`LogicalItemSource::ProviderDefault`，不发思考参数）自动进入它的规格本来挂载的 LLM 用途。不加新机制，只是把“必须手写”改成“默认推导，可手工覆盖”。`provider_server_defaults.md` 里“不因为发现到 LLM 就自动准入”的约定随之修改。

**S4 模型事实只从 Model Driver 读（Q15）。** 上下文长度、是否支持 tool call 等一律来自模型目录，不设通用身份，不给默认值。对不上目录的模型继续留在 `unmatched` 列表里；界面在这份列表上给出两条出路：映射到已有模型（已有的 `model_driver_overrides`），或者等模型目录更新。

**S5 拿不到模型列表时手工填。** endpoint 没有 `/models` 时，向导让用户输入模型名，存进 settings 已有的 `discovery` 字段，不新增存储。填的名字同样要对上模型目录。

**S6 与其它 Provider 的先后不在向导里问（Q14）。** 管理员通常一口气添加一批 Provider，顺序跟他手边的记录有关，逐个确认“优先用这个”没有意义。某个用途用哪家，由已有的路由配置决定。

不做：思考档位开关（自定义 Provider 要思考档位就绑定渠道预设或写实例规则）；价格（默认未知）。

第二步验收建议用一个模型名能对上目录的 OpenAI 兼容端点：填完三项后模型进入用途，在没有别的 Provider 排在前面的用途上能完成对话；换一个目录里没有的模型名，确认它出现在 `unmatched` 里并带着两条出路的提示。

## 7. 裁决记录

owner 2026-11-27 的答复，引号里是原话。

清单与写法：

- **Q1 规则写法。** “按你的建议”。规则级 `protocol_adapter_id` 和 `operations` 的限定写法并存，见 §4.3。
- **Q2 清单处置。** “按你的建议”。§3.2、§3.3 定案。
- **Q3 标准客户端按生态约定实现。** “可以，这类客户端可以加 legacy 前缀”。落法见下面的 N1。
- **Q4 OpenAI 平台拆成几个。** “这个主要从添加‘用户自定义 provider’的角度来看要不要拆。我觉得你的建议应该是符合这个目的”。拆四个；以后判断拆分粒度也用这个角度。
- **Q5 SN。** “SN-API 目前是基于 lite llm 实现的（也相对主流），应该不用发明新的协议客户端”。SN 用标准客户端，第一步删除 `sn-openai`，不另开 SN 服务端的项，见 P1-7。
- **Q6 Gemini。** “按你的建议，最近 gemini 发了新版本，可能需要更多的 Review”。`gemini-generate-content` 放第一步并行做，先 review；Interactions 不再对自定义开放，见 P1-12。
- **Q7 Decision。** “按你的建议”。`typesafe-systemone` 取消对自定义开放。
- **Q8 原生客户端改名。** “按你的建议”。不改 ID，平台名写在客户端清单里。
- **Q9 `responses.background` 上移。** “好，按你的建议”。
- **Q10 云端目录兼容。** “不用，这个版本是 breaking change”。不保留旧格式轨道。同一原则用到了旧数据上，后果列在第一步验收之后。
- **Q11 候选排期。** “按你的建议”。先核实 Azure，其余有客户需求再排，见 §3.5。

第二步：

- **Q12 自定义实例的默认口径。** “按你的建议”。OpenAI 族默认生态口径，见 S1。
- **Q13 自动准入。** “按你的建议”。见 S3。
- **Q14 与托管 SN 的先后。** “不用，实际上管理员会一口气添加一系列 provider，通常和他的 notebook 顺序相关”。向导里不问，见 S6。
- **Q15 通用身份。** “按我们的设计，所有的模型都有 driver meta data 的，从这个 model driver data 里读就好”。不设通用身份，见 S4。
- **Q16 “基本可用”的定义。** “模型目录不认识的名字还是不识别。根本上，AICC 的模型目录是可以在线更新的，但是 AICC 的 API Type 升级和协议客户端升级，需要发版本。（反过来，不是这两种调整，都应该能通过更新配置文件完成更新，包括内置的 provider 的能力升级）”。写成目标第 7 条，并加了一条验收（配置可更新）。

补充项，已裁决：

- **N1 Q3 的落法。** 我提的落法是：官方口径和生态口径分成两个客户端，生态口径的 ID 加 `legacy-` 前缀，即 `legacy-openai-chat-completions` 和 `legacy-claude-messages`；官方口径的客户端保持干净，可以分别对照官方文档和生态约定做 review，也不需要另加“协议选项”这层机制。owner：“N1 按你的建议”。见 §3.1、§3.4、§4.7、P1-5。
- **N2 模型发现行为。** 核对后发现并不是 14 种协议：多数是三种标准列表的同一份实现，真正自成一套的只有 OpenRouter 和 SN（§3.6）。我提了纳入第一步和四件事，owner：“1,2,3,4，都按你的建议”。落在 P1-13：
  1. 列表里的模型事实去掉，模型事实只从 Model Driver 读。我当时的原话是“这些字段可以去掉；但方舟的 `status` 反映的是模型是否正在下线，目录里未必有”，没有说死 `status`。我按“去掉 Kimi 的三个字段和方舟的 `task_type`，保留方舟的 `status`”落的；如果你的意思是连 `status` 一起去掉，`volcengine-ark-models` 也可以删。
  2. SN 改用标准的 `/models`，先确认 SN 的端点能返回标准形状。
  3. TypeSafe 的别名写进规则的 `model_driver_overrides`。
  4. OpenRouter 的厂商前缀映射改成规则里的声明，加一个小的 schema 字段。

实施中要回报 owner、不能自行决定的事项：

- P1-5：并不进生态口径客户端的残余差异（预计是 Kimi 的 `partial`、GLM 的 `tool_stream` 分片归一化、MiniMax 的 `base_resp`、OpenRouter Chat 的 text modality 约束），逐项给出“用配置表达、放弃这项行为、留一个方言客户端”三选一的建议。
- P1-7：SN videos 的两处差异（支持 extend、不能取消）并不进 `openai-videos` 时。
- P1-7：方舟、DeepSeek、SN 的 Responses 是否接受文档输入的核对结果。
- P1-12：Gemini 的 review 结论与 §3.3 不一致时。
- P1-13：SN 的 `/models` 实际返回的形状，以及服务端要不要改；OpenRouter 发现代码里的 Jev 常量能否用规则表达。
- §3.2 里标了“待核对”的两项：方舟图片、OpenRouter videos 是否能算 OpenAI 对应接口的方言。
- 方舟列表里的 `status`：现在按保留处理（N2 第 1 条），owner 没有明说。

## 8. 不做

- 不改模型目录（Model Driver）的结构和匹配链。
- 不改路由和 fallback，那是 `aicc-explicit-fallback-todo.md` 的范围。
- 不做协议自动探测。
- 不动价格体系。
- 不做旧版本兼容。
- §3.5 的候选、§3.6 里没标任务号的存量。
- `aicc-zone-aicc-provider-todo.md` 里计划的 `aicc-krpc` 客户端，实施时按本文的单协议客户端注册，并在客户端清单里归到 C 层。

## 9. 实施记录

2026-11-27 实施，未提交。除 P1-11 外的第一步任务都已落地；P1-11 按本文“可延后，单独合入”留到下一步。

### 9.1 落点

- **P1-0 快照**：`provider/builtin/wire_snapshot_tests.rs`（每个内置 Provider 每个模型的请求、流式、多轮、variant 和按 URL 选的解码夹具，存摘要到 `wire_snapshots.golden`）；`provider/builtin/inventory_snapshot_tests.rs`（库存：API 类型、能力、operation、variant、挂载、价格，724 条，`inventory_snapshots.golden`；另有发现请求快照 `discovery_snapshots.json`，记 URL 和头的名字）。环境变量：`AICC_WIRE_SNAPSHOT_DUMP/UPDATE/DEBUG`、`AICC_INVENTORY_SNAPSHOT_DUMP/UPDATE`、`AICC_DISCOVERY_SNAPSHOT_UPDATE`。基线取自原提交 `37e48f4d9`。
- **绑定与解析**：新文件 `protocol/binding.rs`（`BindingRef` 即 `客户端/operation`、`ProtocolClientSet`、统一解析 `resolve_binding`）。库存、模型表、路由里存的都是带客户端的写法；`SelectedRoute` 拆出 `protocol_adapter_id` 和 `operation`。
- **Catalog**：Known Provider 的 `extra_protocol_adapters: [{id, credential?}]`、`state_namespace`；规则级 `protocol_adapter_id`；Provider Rules 的 `model_driver_vendor_prefixes`。用到这些字段的文件要求 `schema_revision` 2（`catalog/validation.rs`）。`INVENTORY_SCHEMA_VERSION` 升到 3。
- **凭据**（§4.4）：`ProviderProfile::credential_for_client`。顺序：实例显式的 `credential_kind`，默认客户端用 Known Provider 顶层凭据，额外客户端用它声明的 `credential`，最后是客户端自己的描述。
- **命名空间**（§4.5）：`CodecContext.state_namespace`；执行层 `finish_provider_state` 把客户端族的命名空间改成声明的那个，覆盖立即、流式、原生任务三处。
- **settings**：`ProviderSettings.protocol_adapter_id` 改为可选，只有 `custom` 读；托管 SN 不再写它（`kernel/scheduler/src/system_config_builder.rs`）。
- **客户端拆分与迁移**：OpenAI 平台拆成 `openai-embeddings`、`openai-images`、`openai-audio`、`openai-videos`；新增 `legacy-openai-chat-completions`、`legacy-claude-messages`、`rerank`、`openrouter-native`、`gemini-generate-content`；删除 `sn-openai`、`kimi-chat`、`doubao-responses`、`qwen-aggregated-chat`；`glm-media` 改名 `glm-video` 只留视频。各家 metadata 按 §4.6 改写。客户端清单见 `doc/aicc/protocol_clients.md`（由 `protocol/client_catalog_tests.rs` 校验）。
- **P1-12**：`protocol/gemini_generate_content.rs`；review 记录在 `notepads/gemini-generate-content-review.md`。Gemini 官方 profile 的 LLM 仍走 Interactions，向量和 Veo 改引用新客户端，URL 不变；自定义开放的 Gemini 客户端从 `gemini-interactions` 换成 `gemini-generate-content`。
- **P1-13**：`discovery_behavior_id` 只剩 `standard-models`、`catalog-only`、`openrouter-models`、`volcengine-ark-models`、`sn-models`。OpenAI 族一份实现（`OpenAiCompatibleModelsDiscovery`，方舟只多读 `status`），Claude 族按默认客户端是否派生自 `legacy-claude-messages` 决定发不发版本头，Gemini 族一份。TypeSafe 别名进 `typesafe.provider.json` 的 `model_driver_overrides`，`typesafe.rs` 删除。OpenRouter 厂商前缀进 `model_driver_vendor_prefixes`，通用匹配链在 `provider/inventory.rs` 的身份解析里。
- **文档能力**（P1-7 里 agent_tool 一项）：新增特性 `document`（`buckyos_api::features::DOCUMENT`）。`claude-messages`、`openai-responses`、`gemini-interactions`（两个 LLM 绑定）、`gemini-generate-content` 的 LLM 绑定声明它；OpenAI、Claude、Gemini 三个 Model Driver 里带 vision 的 LLM 模型标 `document: true`（Gemma 不标）；OpenRouter、SN 的规则 `remove_features: ["document"]`。`agent_tool` 改读 `route_resolve` 的 `enabled_capabilities`，删除按 Adapter ID 的白名单。
- **P1-8**：`component_adapter_ids` 和跨族分支删除；`typesafe-systemone`、`gemini-interactions` 不再对自定义开放；`legacy-openai-chat-completions`、`legacy-claude-messages` 对自定义开放（分别挂在 openai、claude 规则的 `custom_provider_adapters`）。
- **P1-9**：`buckyos-api`（去掉 `component_adapter_ids`，catalog 视图加 `extra_protocol_adapters`、`state_namespace`）；websdk `src/aicc_client.ts`（dist 已重建）；Desktop AI Center（客户端集合、“客户端 · operation”、规则编辑器读写带客户端的写法和规则级客户端，导出时自动升 `schema_revision` 2；Known Provider 不再发 `protocol_adapter_id`；新文案键 `aiCenter.providers.protocolClients` 的 8 种语言翻译是实施时写的）；`tools/buckyos-agent/commands/ai_provider.ts`；`buckyos_admin` 手册 `references/providers.md`。
- **P1-10**：见 `doc/aicc/` 下各文档的改动。

### 9.2 快照验收

- wire 快照：每个任务后都与基线一致。接受的变化只有两条，都是 §4.7 预期的：`kimi|kimi-k2.6|llm|decode:chat_without_object`、`qwen|MiniMax/MiniMax-M2.1|llm|decode:chat_without_object`，从报错变成正常解码（生态口径补 `object`）。
- 发现请求快照：P1-13 前后一致，包括 Claude 发 `anthropic-version`、MiniMax 不发。
- 库存快照：迁移全程与基线一致。接受的变化只有一项：OpenAI、Claude、Gemini 官方 Provider 上 47 个模型多了 `document` 能力；OpenRouter、SN 不变。
- 测试：`cargo test -p aicc -- --test-threads=1` 822 过；`agent_tool` 372 过（grep 工具的用例要 `rg` 在 PATH 里）；`buckyos-api` 全过；`scheduler` 串行 92 + 21 过（并行时 `system_config_agent` 有 1 到 3 个用例互相干扰，原提交上同样会失败）；websdk `aicc_client` 18 过；Desktop `pnpm check` 通过、相关 Deno 用例 38 过；`ai_provider_test.ts` 2 过。Desktop `ai-center.spec.ts` 5 个里 2 个失败，原因是首页 `/Est\. Cost/` 选择器与现在的文案 “Est. cost” 大小写不一致，以及豆包 TTS 的 mock 缺字段，与本次改动无关。
- 测试数从原来的 806 个用例名变成 822 个。删掉的 15 个都是测已删除代码的（`sn-openai` 视频、Kimi 和千问聚合方言、方言内的命名空间改写、OpenAI 七个 operation 的注册、方舟 `task_type`）。实施中一度误删的 4 个 SN 用例（发现、错误、动态登录、凭据代理）已恢复。

### 9.3 与本文的偏差

- **解析顺序**（§4.3）：没有采用“先在主客户端里选”。实际是在实例全部客户端里取 `default_priority` 最高的正值，同分时主客户端优先；0 只能显式选。理由：OpenAI 拆分后，自定义 `openai-responses` 实例的图片默认要落到 `openai-images`，而主客户端里的 `responses.background` 之类不能抢走 `vision.ocr`。因此普通绑定的默认优先级改为 1，`responses.background`、Chat 做的 ASR、`video.extend` 等用 `explicit_only()` 设 0。
- **`doubao-speech` 的凭据**改为 `named_header`（`x-api-key`），原来是代码里写死。
- **发现**：响应校验统一用宽松口径（不再强制 `object: list/model`）；列表响应上限统一为 8 MiB（原来 OpenAI、Kimi 是 8 MiB，其余 OpenAI 族是 1 MiB）。Known Provider 改了 base 且 base 没有路径时，OpenAI 和 Kimi 的发现地址从 `/v1/models` 变为 `/models`（内置 base 不受影响，快照不变），P1-11 统一 endpoint 时一起定。
- **Kimi 的 thinking 校验放宽**：原来只接受 `keep: "all"`，现在走生态口径，只校验 `type`。
- **`rerank` 未能对自定义开放**（§3.4 写的是“是”）：`custom_provider_adapters` 要求与挂靠的 Known Provider 默认客户端同族，现在没有默认客户端是 rerank 族的 Known Provider。
- **方言比 §3.4 多 3 个**：`glm-chat`、`minimax-messages`、`deepseek-responses` 仍在，见下节。

### 9.4 残余差异（待 owner 定）

- `glm-chat`：只剩流式 `tool_stream` 增量的归一化。内置规则不开 `tool_stream`，内置 Provider 上不会触发。
> AICC的整体原则就是不做流式推理，我们是基于task update来跟踪长任务的变化
- `minimax-messages`：`base_resp` 错误信封（HTTP 200 也可能是错误）；图片按内联字节发送。都表达不成配置。
> 这个要给给修复建议
- `deepseek-responses`：只差 base 无路径时用 `/responses`，等 P1-11。
> 自带的各家官方provider,会配置自己的各个协议客户端的path(从base url派生)。这个我觉得历史上应该很少改变吧，可以先写死。
- Kimi：`partial` 前缀续写丢弃。
> 好
- OpenRouter Chat：原 `openrouter-chat` 的纯文本模态声明丢弃；内置规则没有用 OpenRouter Chat 做 LLM。
> 好
- `qwen-responses`、`openrouter-responses` 的残余写在 `doc/aicc/protocol_clients.md`。另外 Responses 方言里还保留着给上游请求的命名空间写法（`state_coordinate`），与 §4.5 的 `state_namespace` 有重叠，没有动。
> 如果官方已经不支持了，可以删除相关实现
- Kimi、GLM、千问、MiniMax、DeepSeek、OpenRouter、豆包（含 Agent Plan）声明了 `state_namespace`。拒答之类的 provider state 现在也落在这些命名空间下。
> 好

### 9.5 核对结论与报告项

- **自定义实例选媒体客户端做默认时，LLM 报歧义**：默认客户端是 `openai-audio`、`openai-embeddings`、`openai-images`、`openai-videos` 时，它没有 LLM 和 vision 绑定，集合里 `openai-responses`、`openai-chat-completions`、`legacy-openai-chat-completions` 同为 1，按 §4.3 第 3 条报歧义。歧义在库存构建里是错误，网关列表里只要有一个 LLM 模型，整个实例的库存就建不起来。默认客户端是 `openai-responses` 或 `openai-chat-completions` 时没有这个问题。要 owner 定：歧义改成只过滤该模型的这个 API 类型；或在协议族默认规则里写 LLM 的默认绑定；或向导里不让媒体客户端做默认；或维持现状，要求用户写映射。
  - owner 11-27 的处理：常见 Provider 也可能像官方一样由多个协议客户端组合而成，只选一个，其余就要靠猜。以后自定义 Provider 改成只填 endpoint 和 token，客户端经标准探测流程得出；本版本先关闭添加入口（Desktop 向导去掉 Custom Provider 卡片，`buckyos_admin` 手册同步，4 个经向导新建 custom 的 e2e 用例改为 skip；后端和已有实例不变）。另立 TODO：`notepads/aicc-custom-provider-auto-probe-todo.md`，§6 的第二步思路并入那里。

- **SN 视频**：OpenAI 文档写的 Videos API 下线日期是 2026-09-24。官方 `POST /videos/extensions`、`/videos/edits` 用 JSON `video: {id}`，不带 `model`；没有取消接口，`DELETE` 是删除已完成或失败的视频，而 `openai-videos` 把 `DELETE` 当取消用。SN（LiteLLM）的 extend 和 edit 要带 `model`。SN 的 wan-3.0 在规则里是排除的，所以这几条都不可达，没有进 `openai-videos`。
- **Gemini**（P1-12 review，详见 review 文件 §0）：LiteLLM 实现了 Interactions，与 Q6 和 §3.3 “只有官方” 的判断不一致，`gemini-interactions` 是否对自定义开放请 owner 重新确认；Google 已把 `generateContent` 称为 legacy（仍完全支持）；v1 已是稳定版、SDK 默认 v1，新客户端在 base 无路径时仍补 `/v1beta`。
- **直接发文档**：方舟、DeepSeek、SN、OpenRouter 的 Responses 是否接受 `input_file` 都没有核对。现在的结果与迁移前一致：OpenRouter、SN 用规则关掉；方舟、DeepSeek、千问因为 Model Driver 没有 `document` 能力，自然不发。另一个变化：三家官方 Provider 上没有 vision 的 LLM 模型不再直接发文档，改走转换。
- **列表里的模型事实**（N2 第 1 条）：本次没有 Kimi、方舟的 key，没能用真实列表对比前后的库存。目录里 7 个 Kimi 模型都已声明 vision 和 reasoning；方舟 `task_type` 的作用是收窄 API 类型，去掉后由 Model Driver 决定。上线前建议用真实列表各对比一次。
- **SN `/models`**（N2 第 2 条）：没有核对真实 SN 的 `{base}/models`，`sn-models` 保留。
- **Jev 常量**（`openrouter.rs`、`openrouter_decisions.rs`）：渠道 ID 到 `typesafe/jev-1.13.0` 的身份可以写成 `model_driver_overrides`；但 build ID（`canonical_slug`）、上下文 32000、输入输出模态这几项校验是“列表里的字段必须等于某值，否则不可用”，现有规则表达不了，没有改。
- **Moonshot 列表**：官方文档列出 `supports_image_in`、`supports_video_in`、`supports_reasoning`、`context_length` 字段，没有示例值。

### 9.6 没做、没验证

- P1-11：owner 定“先写死”，不做 endpoint 模板，见 §9.7。
- 真实环境没有验证：本机 Zone 上的托管 SN（LLM、图片、ASR、视频）和各家真实调用都没跑；`test/aicc_test/acceptance` 的 T1 没跑（要部署新 AICC 并重启，重启 AICC 要 root）。`cargo build -p aicc --release --target x86_64-unknown-linux-musl` 通过，产物在 `src/target/x86_64-unknown-linux-musl/release/aicc`，没有替换到 `/opt/buckyos/bin/aicc/`。
- 云端目录按新格式发布（Q10）没有做，需要发布时按新格式直接发。
- websdk 重建 dist 时，两个 `aiws_write-*` 的 source map 跟着变了，与本次无关。

### 9.7 第二轮（2026-11-27，按 owner 在 §9.4 的答复）

- **删除 `glm-chat`**：AICC 不做流式推理，进展靠 task update 跟踪（owner）。`tool_stream` 的流式归一化，以及生态口径 Chat 接受的 `tool_stream` 参数一起删掉。GLM 的默认客户端改为 `legacy-openai-chat-completions`。
- **删除 `deepseek-responses`**：owner 定内置 Provider 各客户端的路径从 base 派生、先写死。OpenAI 平台客户端（Responses、图片、音频、向量、视频）的 endpoint 改为 `{base}/{path}`，不再在 base 没有路径时补 `/v1`。内置 Provider 的 base 都已带全路径，只有 DeepSeek 的 base 没有路径，正好得到 `/responses`。DeepSeek 改用 `openai-responses`。代价：自定义实例的 base 不带 `/v1` 时不再自动补（添加入口已关闭，以后由探测流程处理）。
- **`rerank`** 同样改为 `{base}/rerank`，去掉对 OpenRouter `/api/v1` 的假设；URL 不变。
- **P1-11 不做模板**：原生客户端里的路径规则（MiniMax 媒体、百炼媒体、豆包语音的逐 operation 地址、fal 价格地址）按“先写死”保留在各自的客户端和 metadata 里。
- **Responses 方言的命名空间**：删掉 `ProviderStateNamespace`、`EndpointPath` 两个已经没有代码对应的覆盖点声明；命名空间只由 Known Provider 的 `state_namespace` 给出。
- **百炼、OpenRouter 的残余按官方文档核对，都仍受支持，没有删**（核对结果写在 `doc/aicc/protocol_clients.md`）。百炼的 `enable_thinking` 官方已标为将废弃（推荐 `reasoning.effort`）；OpenRouter 的 `route` 已标为废弃（推荐 `provider.sort.partition`），两者都还能用。另有两项是规则关掉、但官方支持的能力，要不要打开请 owner 定：
  - 百炼 Responses 支持 `web_search`（限所列模型），规则里用 `remove_features` 关着。
  - OpenRouter Responses 对所有模型接受 `input_file`（PDF），规则里用 `remove_features: ["document"]` 关着。
- **MiniMax 的修复建议**（依据 MiniMax 官方文档，见 `doc/aicc/protocol_clients.md`）：
  1. 删掉 `base_resp` 处理。官方文档写明 Anthropic 兼容接口的错误统一用 HTTP 状态码加 Anthropic 错误体，流式里是 `event: error`；`base_resp` 只出现在原生 `chatcompletion_v2` 和 OpenAI 兼容接口。余额不足官方没写状态码，第三方记录是 HTTP 402，通用映射已归为 `ProviderRejected`（不重试）。
  2. 图片改走 `legacy-claude-messages` 的默认做法（URL 原样传，Zone 内的对象传字节），与 Claude 官方一致。官方支持 URL 和 base64 两种来源。
  3. 两条都做完，`minimax-messages` 就没有差异了：删除它，MiniMax 的默认客户端改为 `legacy-claude-messages`。wire 快照会有一类变化要接受：URL 形式的图片不再先下载再内联。
  - owner 同意，已实施，见 §9.8。另外两项能力也已按建议打开。
- **顺带修的**：Desktop 的规则导入器不认识 `model_driver_vendor_prefixes`，导入内置 `openrouter.provider.json` 会失败（第一轮引入，已在 `cbd8bbc4f` 里）。现在接受这个字段，导出时把 `schema_revision` 升到 2。
- **验证**：`cargo test -p aicc -- --test-threads=1` 820 过；wire、库存、发现请求三组快照都不变；Desktop `pnpm check`、eslint 通过，相关 Deno 用例 38 过；`ai_provider_test.ts` 2 过。客户端从 25 个减到 23 个。

### 9.8 第三轮（2026-11-27，owner 同意 §9.7 的建议）

- **删除 `minimax-messages`**：MiniMax 的默认客户端改为 `legacy-claude-messages`。`base_resp` 处理只留给 MiniMax 原生媒体接口（`validate_minimax_response` 移到 `minimax_media.rs`）；Anthropic 兼容接口的错误按标准 HTTP 状态码处理。图片随生态口径走 URL 或字节。wire 快照只变了一条：`minimax|MiniMax-M2|llm|decode:messages_base_resp`（带 `base_resp: 1008` 的正常 Messages 响应），原来报错，现在按正常消息解码；没有带 URL 图片的 MiniMax 样例，所以图片那一类没有出现变化。原来借 MiniMax 包装测生态口径 Messages 的 6 个用例移到 `protocol/claude_messages_legacy_tests.rs`，直接测 `legacy-claude-messages`。
- **打开百炼的 `web_search`**：删掉 Qwen 规则里 8 处 `remove_features: ["web_search"]`（这一项原来不起作用：Qwen 的 Model Driver 没有声明这项能力）。按官方列表，在 Qwen Model Driver 里给 Qwen4-Max、Qwen3.8、Qwen3.7、Qwen3.5 系列共 17 个模型声明 `web_search: true`。库存变化：百炼 20 条；OpenRouter 上同名的 17 个 Qwen 模型也有了这项能力（OpenRouter 官方说明 `openrouter:web_search` 对所有模型可用）；SN 上的 7 个 Qwen 模型在 SN 规则里加 `web_search` 到 `remove_features`，保持不变（SN 是否支持没有核对）。
- **打开 OpenRouter 的直接发文档**：删掉 OpenRouter 规则里 22 处 `remove_features: ["document"]`。库存变化：OpenRouter 上 58 个 Claude、Gemini、OpenAI 模型有了 `document` 能力，没有别的差异。
- **验证**：`cargo test -p aicc -- --test-threads=1` 817 过；wire 快照只有上面一条变化，库存快照的变化只有 `web_search` 和 `document` 两项（逐条去掉这两个字段后与原来一致），发现请求快照不变；Desktop `pnpm check` 通过、相关 Deno 用例 38 过。客户端从 23 个减到 22 个，剩下的方言只有 `qwen-responses`、`openrouter-responses`。

