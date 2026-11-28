# AICC 协议客户端清单

一个协议客户端（代码里叫 Adapter，ID 是 `protocol_adapter_id`）对应一种 API 类型：一组 endpoint 路径、请求和响应结构、凭据放在哪个头。这份清单是我们对世界上主流 AI API 类型的理解，也是“改它需要发版本”的全部内容：

- 给某家 Provider 加模型、让它多用一个已有的客户端、改它的规则，只改 `driver_metadata/*.json`，可以在线下发。
- 新增或修改客户端（包括方言）要改代码、发版本，同时改这份清单。

清单由 `src/frame/aicc/src/protocol/client_catalog_tests.rs` 校验：注册表里的客户端必须与表格一一对应，表格由测试生成，每个方言必须在下面有残余差异一节。客户端变化后运行：

```bash
AICC_UPDATE_CLIENT_CATALOG_DOC=1 cargo test -p aicc protocol_client_catalog_document_matches_the_registry
```

层的含义：

- **A 标准，官方口径**：按 API 所属平台的官方文档实现。同一协议族的客户端共用 endpoint 和凭据。
- **A 标准，生态口径**：兼容这种 API 的第三方服务普遍采用的写法（ID 以 `legacy-` 开头）。它基于官方口径的客户端，只覆盖差异。
- **B 官方独有**：只有该平台自己提供的 API，不对自定义 Provider 开放。
- **方言**：某一家对标准客户端的偏离，配置表达不了的部分才留在这里。目标是逐步并入标准客户端或配置。
- **C 原生**：平台自己的媒体或任务协议。

实例的客户端集合：Known Provider 在 `known-providers/*.json` 里声明默认客户端 `protocol_adapter_id` 和有序的 `extra_protocol_adapters`；自定义实例的集合是它选的默认客户端，加上同一协议族里所有对自定义开放的客户端。模型规则里的 operation 写成 `<客户端>/<operation>` 时指定客户端，裸写时属于规则级 `protocol_adapter_id`，再没有就属于实例的默认客户端。

模型发现按默认客户端的协议族选：OpenAI 族 `{base}/models`，Claude 族 `/v1/models`（官方口径发 `anthropic-version`，生态口径不发），Gemini 族 `models`。只有方舟的 `status`、OpenRouter 的目录协议和 SN 的信封另有实现，见 `discovery_behavior_id`：`standard-models`、`catalog-only`、`volcengine-ark-models`、`openrouter-models`、`sn-models`。

<!-- BEGIN GENERATED CLIENTS -->
| 客户端 | 层 | 平台 | 协议族 | 基础客户端 | 对自定义开放 | operation |
| --- | --- | --- | --- | --- | --- | --- |
| `openai-responses` | A 标准，官方口径 | OpenAI | `openai` | — | 是 | `responses.background`<br>`responses.create` |
| `openai-chat-completions` | A 标准，官方口径 | OpenAI | `openai` | — | 是 | `chat.completions.create` |
| `openai-embeddings` | A 标准，官方口径 | OpenAI | `openai` | — | 是 | `embeddings.create` |
| `openai-images` | A 标准，官方口径 | OpenAI | `openai` | — | 是 | `images.edit`<br>`images.generate` |
| `openai-audio` | A 标准，官方口径 | OpenAI | `openai` | — | 是 | `audio.speech`<br>`audio.transcriptions` |
| `openai-videos` | A 标准，官方口径 | OpenAI | `openai` | — | 是 | `videos.create` |
| `claude-messages` | A 标准，官方口径 | Anthropic | `claude` | — | 是 | `messages.create` |
| `gemini-generate-content` | A 标准，官方口径 | Google | `gemini` | — | 是 | `models.embedContent`<br>`models.generateContent`<br>`models.predictLongRunning` |
| `rerank` | A 标准，官方口径 | Cohere / Jina 口径 | `rerank` | — | 否 | `rerank.create` |
| `legacy-openai-chat-completions` | A 标准，生态口径 | OpenAI 兼容生态 | `openai` | `openai-chat-completions` | 是 | `chat.completions.create` |
| `legacy-claude-messages` | A 标准，生态口径 | Anthropic 兼容生态 | `claude` | `claude-messages` | 是 | `messages.create` |
| `gemini-interactions` | B 官方独有 | Google | `gemini` | — | 否 | `interactions.background`<br>`interactions.create` |
| `qwen-responses` | 方言 | 阿里云百炼 | `openai` | `openai-responses` | 否 | `responses.create` |
| `openrouter-responses` | 方言 | OpenRouter | `openai` | `openai-responses` | 否 | `responses.create` |
| `deepseek-responses` | 方言（待 P1-11 删除） | DeepSeek | `openai` | `openai-responses` | 否 | `responses.create` |
| `glm-chat` | 方言（残余待 owner 定） | 智谱 | `openai` | `legacy-openai-chat-completions` | 否 | `chat.completions.create` |
| `minimax-messages` | 方言（残余待 owner 定） | MiniMax | `claude` | `legacy-claude-messages` | 否 | `messages.create` |
| `fal-queue` | C 原生 | fal | `fal` | — | 否 | `queue.submit` |
| `minimax-media` | C 原生 | MiniMax | `minimax` | — | 否 | `image_generation.create`<br>`music_generation.create`<br>`speech_to_text.create`<br>`t2a.create`<br>`video_generation.create`<br>`video_generation.v2.create` |
| `doubao-media` | C 原生 | 火山方舟 | `doubao` | — | 否 | `ark.contents.generate`<br>`ark.embeddings.multimodal`<br>`ark.images.generate` |
| `doubao-speech` | C 原生 | 豆包语音 | `doubao` | — | 否 | `asr.recognize.flash`<br>`asr.recognize.idle.submit`<br>`asr.recognize.submit`<br>`tts.unidirectional` |
| `qwen-media` | C 原生 | 阿里云百炼 | `qwen` | — | 否 | `dashscope.image_edit`<br>`dashscope.image_generation`<br>`dashscope.image_synthesis`<br>`dashscope.video_synthesis` |
| `glm-video` | C 原生 | 智谱 | `glm` | — | 否 | `videos.generate` |
| `openrouter-native` | C 原生 | OpenRouter | `openrouter` | — | 否 | `audio.speech`<br>`chat.completions.create`<br>`decisions.create`<br>`images.generate`<br>`videos.create` |
| `typesafe-systemone` | C 原生 | TypeSafe | `typesafe-systemone` | — | 否 | `systemone.evaluate` |
<!-- END GENERATED CLIENTS -->

## 方言的残余差异

### `qwen-responses`

百炼 Responses 兼容接口的差异：`enable_thinking` 放进请求体；`session_cache` 参数改成 `x-dashscope-session-cache` 请求头；视频类的文档输入改写成视频输入。`web_search` 能力由规则的 `remove_features` 关掉，不在客户端里。

### `openrouter-responses`

OpenRouter Responses 的差异：OpenRouter 自己的请求参数（`provider`、`plugins`、`models`、`route`、`cache_control` 等）透传进请求体；`web_search` 工具类型改写成 `openrouter:web_search`；用量里的费用按 USD 读。直接发文档没有核对，规则用 `remove_features: ["document"]` 关掉。

### `deepseek-responses`

只差 endpoint：base 没有路径时用 `/responses`，而不是官方客户端的 `/v1/responses`。P1-11 用 base 推算表达后删除。

### `glm-chat`

基于 `legacy-openai-chat-completions`。只剩流式里 `tool_stream` 增量参数的归一化（GLM 把工具参数分多块发）。内置规则没有开 `tool_stream`，这段在内置 Provider 上不会触发；是否删除待 owner 定。

### `minimax-messages`

基于 `legacy-claude-messages`。剩两处：响应里的 `base_resp` 错误信封（HTTP 200 也可能是错误），以及图片按内联字节发送（不转 URL）。两处都不能用配置表达，待 owner 定。
