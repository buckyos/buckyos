# `gemini-generate-content` 实施前 review（P1-12）

核对日期：2026-11-27。来源都是当天读的官方文档；`ai.google.dev/gemini-api/docs/*` 和 `ai.google.dev/api/*` 现在都 301 到 `aistudio.google.com/docs/*`、`aistudio.google.com/api-reference/*`，下面写的是跳转后的地址。每个页面加 `.md` 能拿到原文（`https://aistudio.google.com/llms.txt` 有索引）。

## 0. 与 TODO §3.3 判断的出入（先报告 owner）

TODO 的两条前提：

1. “市面上的 Gemini 兼容网关实现的是 `generateContent`”：**成立**。核对过的网关都实现 `models/{model}:generateContent` 与 `:streamGenerateContent?alt=sse`（§3）。
2. “Interactions 只有 Google 自己在用”（Q6 把 `gemini-interactions` 归 B 层、不对自定义开放的依据）：**已经不成立**。
   - LiteLLM 代理实现了 Interactions：`/v1beta/interactions` 及 get / cancel / delete；`gemini`、`vertex_ai` 原样转发，其它所有 Provider 桥接到 LiteLLM 的 `/responses` 再转回来（https://docs.litellm.ai/docs/interactions）。
   - 按 LiteLLM 文档，Vertex AI 也有自己的 Interactions API。
   - Google GA 公告点名 LiteLLM 是已经集成 Interactions 的合作方，并说正与生态伙伴合作，让 Interactions 成为第三方 SDK 和库的默认接口（https://blog.google/innovation-and-ai/technology/developers-tools/interactions-api-general-availability）。
   - SN 基于 LiteLLM（TODO §3.1 规则 4），所以 SN 理论上也能提供 Interactions。

   这不影响新增 `gemini-generate-content`。但 Q6 “`gemini-interactions` 不对自定义开放” 需要 owner 重新确认：现在接 LiteLLM 代理的自定义 Provider 是可以走 Interactions 的。

另有一条 TODO 没写到的现状：Google 已经把 `generateContent` 称为 **legacy**（§1）。它仍然完全支持，主线模型也继续上它。所以 “Gemini 族的标准客户端是 `generateContent`” 这个判断目前成立，但方向上 Google 在推 Interactions。

## 1. Interactions 与 generateContent 的现状

- Interactions：“As of June 2026, it is Generally Available and the recommended interface for all new projects.”（https://aistudio.google.com/docs/interactions-overview）
- generateContent：“While it is now considered legacy, the original generateContent API remains fully supported.”（同上）。GA 公告的说法是 “the legacy generateContent API remains fully supported” 并且 “will continue to receive new mainline Gemini models”。
- 新东西：“new models and capabilities beyond the core mainline family, along with new agentic capabilities and tools, will launch on the Interactions API.”（同上）。例如 Genie 4（11-19）只在 Interactions 上；Tool retrieval（11-19，Preview）只在 Interactions 上。
- 只有 generateContent 有、Interactions 还没有的：Batch API、Python 自动函数调用、显式缓存（同上，页面末尾）。
- 迁移指南没有给 generateContent 的下线时间表：“While `generateContent` remains fully supported, we recommend the Interactions API for all new development.”（https://aistudio.google.com/docs/migrate-to-interactions）
- 给 agent 的文档索引写的是：“Use the Interactions API (`client.interactions`) as the default across all workflows. `generateContent` is still supported for existing integrations, but is considered a legacy API.”（https://aistudio.google.com/llms.txt）

## 2. 最近一次版本更新

owner 说的 “Gemini 最近发了新版本”，对照 release notes（https://aistudio.google.com/docs/changelog）和 API 版本说明（https://aistudio.google.com/api-reference/api-versions），相关的是：

- **v1 成为稳定版，SDK 默认改 v1。** “The **Interactions API** and its core features are generally available in `v1`.” “Starting with the November 2026 major releases, the Google Gen AI SDKs call `v1` unless you set a version. Earlier releases defaulted to `v1beta`.” 能力表里 generateContent 的核心能力（函数调用、结构化输出、thinking、system instruction、服务档位、各内置工具）两个版本都有；只在 v1beta 的有：音频输出（speechConfig）、转写配置、Live API、显式上下文缓存、Agents、Webhooks。
- **Gemini 4 系列 GA**：Argon（10-13）、Carbon（11-10）、Hydrogen（11-12）、Neon（11-18）。快速入门用的仍是 `generateContent`。
  - `thinkingLevel` 对 Gemini 4 是必需的，Gemini 3 推荐；取值 `MINIMAL`、`LOW`、`MEDIUM`、`HIGH`、`XHIGH`（`XHIGH` 只有 Neon、Hydrogen；Gemini 4 不支持 `minimal`）。Gemini 4 上 `thinkingBudget` 已弃用，单独传会映射到最近的档位，和 `thinkingLevel` 一起传返回 400（https://aistudio.google.com/docs/thinking?generate-content=true）。
  - **Long decode**：新的 finish reason `CONTINUATION`，候选里带 `continuationToken`，请求体新增顶层 `continuationToken`（必须原样重发 `contents`）。单次请求上限 64K；**Carbon、Argon 的非流式 REST 调用上限 32K，`maxOutputTokens` 大于 32768 又没带 token 会返回 400**。流式在 32K（Carbon、Argon）或 64K 之后每 5000 token 发一次 checkpoint token（https://aistudio.google.com/docs/long-decode?generate-content=true）。
- **采样参数**：`temperature`、`topP`、`topK` 从 3.6 Flash 起弃用并被静默忽略，文档说 “In future model generations, supplying these parameters returns an HTTP 400 error.”（https://aistudio.google.com/docs/whats-new-gemini-3.6?generate-content=true）
- **预填 model 轮**：“If the last non-empty turn in the request is a `model` turn, the API returns a `400` error.”（同上）。`candidateCount` 在 Gemini 3 及以后不支持。
- 11-02：`gemini-2.5-pro`、`gemini-2.5-flash` 弃用，12-01 起自动转到 3.8 Flash / 3.5 Flash-Lite；列出的 breaking change 是函数调用与 `thought_signature` 校验、`thinking_level` 取代 `thinking_budget`、去掉采样参数和预填 model 轮。
- 10-26：付费档新项目的 generateContent 默认 `store=true`（请求体新增 `store`）。
- 11 月：函数响应的 `parts` 接受音频、视频、Office 文件和 `fileData`。

## 3. 兼容网关实现的是哪一套

| 网关 | generateContent | Interactions | 来源 |
| --- | --- | --- | --- |
| LiteLLM 代理 | `/v1beta/models/{model}:generateContent`、`:streamGenerateContent`，示例用 Bearer | 有，`/v1beta/interactions`，非 Gemini Provider 桥接到 Responses | https://docs.litellm.ai/docs/generateContent 、https://docs.litellm.ai/docs/interactions |
| agentgateway | `generateContent`、`streamGenerateContent`，原样转发，不转换到别家 | 文档未提 | https://agentgateway.dev/docs/standalone/latest/documentation/llm/api-types/gemini/ |
| CometAPI | `/v1beta/models/{model}:generateContent`、`:streamGenerateContent?alt=sse`；`x-goog-api-key` 和 Bearer 都接受 | 文档未提 | https://apidoc.cometapi.com/api/text/gemini-generating-content |
| Cloudflare AI Gateway | 按路径转发，示例是 `.../google-ai-studio/v1/models/{model}:generateContent` | 文档未提（路径转发，理论上能过） | https://developers.cloudflare.com/ai-gateway/usage/providers/google-ai-studio/ |

结论：生态通行的是 v1beta 路径的 `generateContent` + `x-goog-api-key`。只有 LiteLLM 两套都实现。

## 4. generateContent 的 wire 形状（实施依据）

来源：https://aistudio.google.com/api-reference/generate-content/generate-content 、https://aistudio.google.com/api-reference/generate-content/stream-generate-content

- 端点：`POST https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent`；流式 `POST .../models/{model}:streamGenerateContent?alt=sse`，SSE 每个 `data:` 是一个 `GenerateContentResponse` 分片，没有事件名，也没有 `[DONE]`。v1 有同名端点（参考页 `?api-version=v1`）。
- 凭据：`x-goog-api-key: $GEMINI_API_KEY`（文档示例也有 `?key=`）。
- 请求体顶层：`contents`（必填）、`systemInstruction`（Content，只收文本）、`tools`、`toolConfig`、`includeServerSideToolInvocations`、`safetySettings`、`generationConfig`、`cachedContent`、`serviceTier`（`standard` / `flex` / `priority`）、`store`、`continuationToken`、`labels`。
- `Content`：`role`（`user` / `model`）、`parts`。`Part`：`text`、`inlineData`（`mimeType`、`data`、`displayName`）、`fileData`（`fileUri`、`mimeType`、`displayName`）、`functionCall`（`id`、`name`、`args`）、`functionResponse`（`id`、`name`、`response`（对象，必填）、`parts`、`willContinue`、`scheduling`）、`executableCode`、`codeExecutionResult`、`videoMetadata`、`partMetadata`、`mediaResolution`、`thought`（布尔）、`thoughtSignature`（字符串）。
- `Tool`：`functionDeclarations`、`googleSearch`、`googleSearchRetrieval`、`codeExecution`、`urlContext`、`fileSearch`、`computerUse`、`mcpServers`、`googleMaps`。`FunctionDeclaration`：`name`、`description`、`parameters`（OpenAPI 子集）或 `parametersJsonSchema`（JSON Schema，二者互斥）、`response` / `responseJsonSchema`、`behavior`。`toolConfig.functionCallingConfig.mode`：`AUTO`、`ANY`、`NONE`、`VALIDATED`，另有 `allowedFunctionNames`。
- `generationConfig`：`candidateCount`、`stopSequences`（最多 5 个）、`maxOutputTokens`、`temperature`、`topP`、`topK`、`seed`、`responseMimeType`（`text/plain`、`application/json`、`text/x.enum`）、`responseJsonSchema`、`presencePenalty`、`frequencyPenalty`、`responseLogprobs`、`logprobs`、`responseModalities`、`speechConfig`、`thinkingConfig`（`includeThoughts`、`thinkingBudget`、`thinkingLevel`）、`imageConfig`、`mediaResolution`、`responseFormat`、`translationConfig`、`audioTranscriptionConfig` 等。结构化输出指南用的是 `responseMimeType: application/json` + `responseJsonSchema`（https://aistudio.google.com/docs/structured-output?generate-content=true）；参考页里 `responseSchema` 已经不列了。
- 响应：`candidates`、`promptFeedback`（`blockReason`：`SAFETY`、`OTHER`、`BLOCKLIST`、`PROHIBITED_CONTENT`、`IMAGE_SAFETY`）、`usageMetadata`、`modelVersion`、`responseId`、`modelStatus`。`Candidate`：`content`、`finishReason`、`finishMessage`、`safetyRatings`、`citationMetadata`、`groundingMetadata`、`urlContextMetadata`、`continuationToken` 等。
- `finishReason`：`STOP`、`MAX_TOKENS`、`SAFETY`、`RECITATION`、`LANGUAGE`、`OTHER`、`BLOCKLIST`、`PROHIBITED_CONTENT`、`SPII`、`MALFORMED_FUNCTION_CALL`、`IMAGE_SAFETY`、`IMAGE_PROHIBITED_CONTENT`、`IMAGE_OTHER`、`NO_IMAGE`、`IMAGE_RECITATION`、`UNEXPECTED_TOOL_CALL`、`TOO_MANY_TOOL_CALLS`、`MISSING_THOUGHT_SIGNATURE`、`MALFORMED_RESPONSE`、`ESCALATION`、`PUP_LIMITED_DISABLED`、`CONTINUATION`。
- `usageMetadata`：`promptTokenCount`（含缓存部分）、`cachedContentTokenCount`、`candidatesTokenCount`、`toolUsePromptTokenCount`、`thoughtsTokenCount`、`totalTokenCount`（= prompt + tool-use prompt + thoughts + candidates）、`promptTokensDetails` / `cacheTokensDetails` / `candidatesTokensDetails` / `toolUsePromptTokensDetails`（`ModalityTokenCount`：`modality`、`tokenCount`）、`serviceTier`。Long decode 的多跳结果另有 `initialPromptTokenCount`。
- 思考与签名（https://aistudio.google.com/docs/thought-signatures?generate-content=true 、https://aistudio.google.com/docs/function-calling?generate-content=true）：
  - generateContent 没有单独的 thought 块；`includeThoughts: true` 时思考摘要是 `thought: true` 的文本 part。签名挂在任意 part 上：有函数调用时在第一个 `functionCall` part（并行调用只有第一个有），没有时在最后一个 part。
  - Gemini 3、4：当前轮内每一步第一个 `functionCall` part 必须带回签名，否则 400。非函数调用 part 的签名建议带回，不强制。
  - 规则：签名必须放回原来的 part；有签名的 part 不能和没签名的合并，两个有签名的 part 也不能合并。
  - 并行调用的回放顺序必须是 “FC1+签名, FC2, FR1, FR2”，交错会 400。
  - 流式时签名可能出现在一个 `text` 为空的 part 里，要读到 `finishReason` 为止。
  - 需要注入不是模型生成的函数调用时（例如把别的模型的轨迹转过来），可以用假签名 `"context_engineering_is_the_way_to_go"` 或 `"skip_thought_signature_validator"` 跳过校验。
  - Gemini 3 起 `functionCall.id` 总是返回，`functionResponse` 要带同一个 `id`；2.5 及兼容网关可能不返回。

## 5. 实施取舍（按上面的结论）

- base 没有路径时补 `/v1beta`。理由：Known Provider 的 base 就是 `.../v1beta`，网关和 REST 示例都用 v1beta，现有 `gemini.rs` 的 `api_endpoint` 也是这样。v1 已是稳定版、SDK 默认 v1，是否改默认值交 owner 定；base 写了路径时原样使用，填 `.../v1` 也能用。
- 工具用 `parametersJsonSchema`，结构化输出用 `responseJsonSchema`：canonical 的 schema 是 JSON Schema，这两个是官方的 JSON Schema 字段。风险：做格式转换的网关可能只认 `parameters` / `responseSchema`，我没有逐家核对。
- 思考签名按原 part 回放：响应里整段 `content`（`role` + `parts`）存成一条 `gemini` 命名空间的 provider state，回放给同一来源时原样发回，不从 canonical 块重建。流式累积时只合并相邻、`thought` 标记相同、且都没有签名的文本 part。
- canonical 历史里没有原生状态的函数调用（来自别的模型或别的实例），给每个 model 内容的第一个 `functionCall` 加官方的假签名 `skip_thought_signature_validator`，否则 Gemini 3、4 会 400。
- `functionCall.id` 缺失时按位置生成 call_id；回放原生 part 时 `functionResponse.id` 只在原 part 有 `id` 时才带。
- Long decode 不做：`CONTINUATION` 作为 finish reason 原样返回，`continuationToken` 不处理。Carbon、Argon 非流式 32K 的限制要靠规则里的 `capability_limits` 表达（现有 Gemini 规则给 Argon、Carbon 写的是 65536，如果以后这两个模型的立即模式改走本客户端，会被 400）。
- 采样参数调用方给了才发，按规则删（与 Claude 客户端一致；Gemini 规则已经删了 `/temperature`、`/top_p`）。

## 6. 实施记录（2026-11-27，未提交）

- 新文件 `src/frame/aicc/src/protocol/gemini_generate_content.rs`：`gemini-generate-content`（族 `gemini`、Preview、`x-goog-api-key`、不探测）。`models.generateContent` 绑定 `llm`（Immediate + Stream）、`vision.ocr`、`vision.caption`，优先级是 `OperationBinding::new` 的默认值 0；视觉两项复用 Chat Completions 的降级（`vision_ocr_as_chat` 等）。
- `models.embedContent`、`models.predictLongRunning` 的描述和 codec 从 `gemini_interactions_adapter()` 移到这里，codec 仍在 `gemini.rs`（改为 `pub(super)`），wire 不变，`predictLongRunning` 优先级仍是 20。`gemini-interactions` 只剩 `interactions.create`、`interactions.background`。
- 注册在 `protocol/plugins/core.rs`，导出在 `protocol/mod.rs`。
- 测试：新文件 9 个单测；改了断言旧归属的用例（`gemini.rs` 的描述与 embedding / Veo 用例、`call/mod.rs` 的 golden 绑定与多 operation 默认、`provider/tests.rs` 两个默认选择用例、`provider/builtin/registry.rs` 的 Adapter 计数与 resource form）。`doc/aicc/provider_operation_bindings.md` 的四行改到新客户端，`provider_ui_backend_mapping.md` 删掉已不成立的视频一行。`cargo test -p aicc -- --test-threads=1`：820 通过 0 失败（基线 811 / 0）。
- **过渡期缺口**：Gemini Known Provider 的默认客户端仍是 `gemini-interactions`，规则里的 `models.embedContent`、`models.predictLongRunning` 在这个客户端里已经不存在。在 P1-1（绑定带客户端）、P1-2（`extra_protocol_adapters`）接上之前，官方 Gemini 实例的向量和 Veo 视频会在库存里被过滤成 `unregistered_binding`（已用 `gemini-embedding-2-preview` 核实）。golden 测试里暂时按 “Gemini 的 `models.*` 归新客户端” 记录 Adapter 列。
- 没有做：真实 API 调用（没有 key）；long decode；把 `gemini-generate-content` 写进任何 Provider 的 `custom_provider_adapters`（`driver_metadata` 不在本次范围）。
