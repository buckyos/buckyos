# AICC TS Mock Provider 组件 Part 2

在 Part 1 基础上扩展 Claude-like、Gemini-like、fal-like 协议、streaming、多模态和异步任务接口。

本文档是拆分后的自包含验收任务文档。实现或评审本任务时，以本文档和 README 中列出的依赖文档为准。

## 1. Provider 协议覆盖

| Provider | 输入格式 | 输出格式 | Streaming / 异步 | Mock 重点 |
|---|---|---|---|---|
| OpenAI | Responses API、image generation/edit、audio transcription/speech、embedding | text、tool calls、JSON schema、image/audio artifact、usage | SSE delta 归并；图片/音频直接 artifact | tool call、JSON schema、vision content part、rate limit、context too long |
| Claude | Messages API、content blocks、tool use、vision block | text block、tool_use、stop_reason、usage | SSE event stream 归并 | content block 转换、tool schema、vision fallback、overloaded/rate limit |
| Google Gemini | `generateContent`、多模态 parts、embedding、image/video/audio | candidates、function_call、safety、media outputs | streamGenerateContent / 长任务 operation | parts 映射、safety block、multimodal embedding space、video operation |
| OpenAI-compatible / OpenRouter | Chat completions 或 responses-like | OpenAI-like，但字段可能缺失或扩展 | SSE 兼容差异 | 兼容字段缺失、模型名映射、provider-specific error |
| fal | 图片/音频/视频工具型任务 | artifact URL / operation status | 异步 submit + poll | upscale、bg_remove、audio.enhance、video.upscale、operation timeout |
| SN AI Provider | AICC `settings.sn-ai-provider`，经 SN 转发到兼容模型服务 | OpenAI-like 或 SN 归一响应 | 由 SN AI Provider 能力决定 | 无普通 API key 参数、`runtime_session` / SN 链路可达性、provider instance 命名、usage / trace / free credit 归因 |

P0 Provider 最小集合按 `aicc_provider_plan.md`：

- `openai.rs`
- `claude.rs`
- `gemini.rs` / `google-gemini` driver（代码中保留历史拼写，配置与 metadata 统一按 Google Gemini 语义验收）
- `fal.rs`

OpenRouter 在 Mock 和 Provider adapter 单测中仍可作为 P1 optional provider；在 L4 gateway 发布强覆盖验收中纳入 Provider 覆盖矩阵，用于验证 OpenAI-compatible 长尾模型、成本 fallback 和兼容性。普通开发验收缺少 OpenRouter key 时应 skipped，不阻塞 P0。

### 1.1 L4 真实 Provider、逻辑目录与物理模型矩阵

L4 不再按“每 Provider 一条用例”或“Provider × model 一条用例”收敛，而是由 runner 在测试开始时读取当前临时 group 中的 `models.list` / Provider inventory / 逻辑目录配置，生成完整覆盖矩阵：

```text
case_set = {
  api_type in canonical ApiType
} x {
  method in methods_supporting(api_type)
} x {
  logical_path in standard_logical_paths where logical_path.api_type == api_type
} x {
  provider in enabled_official_providers
} x {
  model in provider.supported_models
    where model.api_types contains api_type
      and model is mounted to logical_path or admitted by logical_path min_line
}
```

矩阵来源：

1. `canonical ApiType` 以 `src/frame/aicc/src/model_types.rs` 中的 `ApiType` 序列化值为准。当前 `llm` 是 canonical api_type，`llm.chat` 是 method；`vision.ocr`、`vision.caption`、`vision.detect`、`vision.segment` 是 api_type，但其标准逻辑目录路径在内置树中是 `image.ocr`、`image.caption`、`image.detect`、`image.segment`。
2. `methods_supporting(api_type)` 以本文件 §7 Method 验收清单和 `aicc_api设计.md` 为准。一个 api_type 可以对应多个 method，例如 `llm` 需要覆盖 `route.resolve`、`chat.completions.create`、`helper.llm_chat`、`llm.chat`、`llm.completion` 中适用的调用形态。
3. `standard_logical_paths` 以当前运行版本加载的 `LocalLogicalTreeConfig.logical_definitions`、`SessionConfig.logical_tree` 全部可寻址节点和 `models.list` 暴露的逻辑目录为准；该配置默认来自 `build_builtin_local_logical_tree_config()`，并可被 system_config 中的官方 routing 配置叠加。runner 必须把最终生效的标准逻辑目录路径写入报告，并标明每个路径的来源、继承到的 api_type、items、fallback 和 admission 结果。
4. `enabled_official_providers` 发布强覆盖默认至少包含 `openai`、`fal`、`google-gemini`、`claude`、`openrouter`、`sn-ai-provider`；如果官方配置或本次发布基线新增 Provider driver，必须自动纳入矩阵或在报告中标记为未覆盖缺口。
5. `supported_models` 以 AICC 实际注册并可被 `models.list` 观察到的模型为准，包含精确模型名、provider instance、`api_types`、`logical_mounts`、capabilities、health 和 pricing 摘要。

矩阵生成规则：

1. runner 必须先生成 `api_type × method × logical_path × provider × model` 的候选矩阵，再按模型实际能力、逻辑目录 `min_line`、`disable_line`、`mount_mode`、health、quota、policy 和 key 可用性决定 `planned` / `skipped` / `not_applicable`。
2. `skipped` 只用于环境缺失或凭据缺失；模型不支持该 api_type、未挂载到该逻辑目录或不满足 `min_line` 时，应记录为 `not_applicable`，不能混入 skipped 通过率。
3. 每个 `planned` 用例必须执行两段验证：逻辑模型段用 `logical_path` 发起路由或 helper/legacy 调用，断言 route trace 中的 `requested_model_type=logical`、`resolved_logical_path`、`selected_exact_model` 和 provider；物理模型段使用同一个 `selected_exact_model` 或矩阵中的 exact model 发起 typed inference / exact model 调用，断言 `requested_model_type=exact`、不发生隐式 fallback、usage 和 trace 正确。
4. 如果某个 method 只允许 exact model，例如 typed inference，逻辑模型段必须拆成 `route.resolve(logical_path)`，再把结果传给该 method；如果某个 legacy/helper method 接受逻辑模型名，则必须直接用逻辑路径调用一次。
5. 同一个 Provider 下同一个物理模型如果支持多个 `api_types`，不得只用一条“代表性 workflow”替代全部 api_type 覆盖；可以把昂贵能力合并到同一 workflow 中执行，但报告必须保留每个 `api_type × method × logical_path × provider × model` 维度的覆盖状态。
6. Provider 已启用但没有任何可用模型时，生成一个 `skipped` 诊断用例，原因记为 `provider_has_no_models`。
7. `sn-ai-provider` 不需要普通 API key；如果临时 group 的 `settings.sn-ai-provider` 没有注册成功，应判为环境或配置失败，而不是 key 缺失。
8. `openai`、`fal`、`google-gemini`、`claude`、`openrouter` 缺少对应 API key 时，该 Provider 的全部真实模型用例标记为 `skipped`，并在报告中按 Provider 汇总；发布强覆盖模式可在 preflight 直接失败。
9. 每个真实模型用例最多执行 3 次 attempt：首次失败后只重跑同一个 `api_type × method × logical_path × Provider × model` 用例 2 次；任意一次 attempt 成功则该用例最终为 `passed`。
10. attempt 失败原因必须全部保留在报告中，最终成功的用例也要记录之前失败 attempt 的 `failure_class`、错误码和耗时，便于分析不稳定性。

## 2. Mock Provider 契约

Mock Provider 必须提供统一、确定、低成本的行为控制能力。

### 2.1 通用原则

- 固定 seed、固定响应、固定 usage、固定 cost、固定 latency bucket。
- 不访问外网，不依赖真实 API key。
- 所有非确定行为必须由测试显式配置。
- 支持 provider health、quota、pricing、capabilities 的动态切换。
- 支持非结构化输出策略：小结果 inline，大结果 `named_object` artifact。
- 支持 Provider-native streaming 模拟：按固定 chunk 输出，Adapter 聚合后写最终 `AiResponseSummary`，中间 progress 写 task data。

### 2.2 行为控制

Mock 行为可通过 request `payload.options.mock_behavior`、测试专用 header 或 Mock 管理接口控制。推荐字段：

```json
{
  "mock_behavior": {
    "scenario": "success",
    "latency_ms": 20,
    "stream_chunks": ["hello", " ", "world"],
    "error": null,
    "usage": {
      "input_tokens": 10,
      "output_tokens": 5,
      "total_tokens": 15
    },
    "artifact_size": "small"
  }
}
```

必须支持的 `scenario`：

| scenario | 含义 |
|---|---|
| `success` | 固定成功 |
| `stream_success` | 固定 streaming chunks，最终成功 |
| `async_success` | 返回 running，随后写最终 task result |
| `rate_limit` | 返回 429 / provider rate limit |
| `quota_exhausted` | 返回 quota exhausted |
| `provider_5xx` | 返回 Provider 5xx |
| `timeout` | 超时 |
| `malformed_response` | 返回格式错误 |
| `missing_usage` | 成功响应缺 usage |
| `invalid_resource` | 资源无效 |
| `safety_blocked` | Provider 内容安全拒绝 |
| `health_unavailable` | Provider health 不可用 |

## 3. Mock Provider HTTP 接口约定

L3 TypeScript Mock Provider 应尽量模拟 Provider 原生接口，而不是只模拟 AICC 内部 trait。这样可以测试 Provider Adapter 的真实协议转换。

### 3.1 管理接口

Mock Provider 需要提供测试管理接口：

| Method | Path | 说明 |
|---|---|---|
| `GET` | `/__mock/health` | 健康检查 |
| `POST` | `/__mock/reset` | 清空请求记录和动态状态 |
| `POST` | `/__mock/scenario` | 设置默认 scenario 或按 request id 设置 scenario |
| `POST` | `/__mock/provider_state` | 设置 health、quota、latency、capabilities |
| `GET` | `/__mock/requests` | 返回已收到的脱敏请求记录 |
| `GET` | `/__mock/metrics` | 返回调用次数、错误次数、stream chunk 计数 |

管理接口不应暴露真实 key、session token 和原始敏感资源内容。

### 3.2 OpenAI-like 接口

建议支持：

| Method | Path | 覆盖能力 |
|---|---|---|
| `POST` | `/v1/responses` | `llm.chat`、tool call、JSON schema、stream |
| `POST` | `/v1/chat/completions` | OpenAI-compatible / legacy 兼容 |
| `POST` | `/v1/embeddings` | `embedding.text` |
| `POST` | `/v1/images/generations` | `image.txt2img` |
| `POST` | `/v1/images/edits` | `image.img2img`、`image.inpaint` |
| `POST` | `/v1/audio/transcriptions` | `audio.asr` |
| `POST` | `/v1/audio/speech` | `audio.tts` |

### 3.3 Claude-like 接口

建议支持：

| Method | Path | 覆盖能力 |
|---|---|---|
| `POST` | `/v1/messages` | `llm.chat`、content block、tool use、vision |
| `POST` | `/v1/messages?stream=true` | SSE streaming |

### 3.4 Gemini-like 接口

建议支持：

| Method | Path | 覆盖能力 |
|---|---|---|
| `POST` | `/v1beta/models/{model}:generateContent` | `llm.chat`、multimodal parts、function call |
| `POST` | `/v1beta/models/{model}:streamGenerateContent` | streaming |
| `POST` | `/v1beta/models/{model}:embedContent` | `embedding.text`、`embedding.multimodal` |
| `GET` | `/v1beta/operations/{operation}` | video / long running operation |

### 3.5 fal-like 接口

建议支持：

| Method | Path | 覆盖能力 |
|---|---|---|
| `POST` | `/fal-ai/esrgan` | `image.upscale` |
| `POST` | `/fal-ai/imageutils/rembg` | `image.bg_remove` |
| `POST` | `/fal-ai/deepfilternet3` | `audio.enhance` |
| `POST` | `/fal-ai/video-upscaler` | `video.upscale` |
| `GET` | `/queue/requests/{request_id}/status` | 异步状态 |
| `GET` | `/queue/requests/{request_id}` | 异步结果 |

