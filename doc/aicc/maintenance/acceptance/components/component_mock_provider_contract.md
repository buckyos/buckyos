# AICC Mock Provider 契约组件

定义 Mock Provider 的通用原则、行为控制、配置样例和 HTTP 接口契约。

本文档是拆分后的自包含验收任务文档。实现或评审本任务时，以本文档和 README 中列出的依赖文档为准。

## 1. Mock Provider 契约

Mock Provider 必须提供统一、确定、低成本的行为控制能力。

### 1.1 通用原则

- 固定 seed、固定响应、固定 usage、固定 cost、固定 latency bucket。
- 不访问外网，不依赖真实 API key。
- 所有非确定行为必须由测试显式配置。
- 支持 provider health、quota、pricing、capabilities 的动态切换。
- 支持非结构化输出策略：小结果 inline，大结果 `named_object` artifact。
- 支持 Provider-native streaming 模拟：按固定 chunk 输出，Adapter 聚合后写最终 `AiResponseSummary`，中间 progress 写 task data。

### 1.2 行为控制

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

## 2. Mock Provider 配置样例

L3 TypeScript Mock Provider 建议使用 TOML 或 JSON 配置。示例：

```toml
[server]
host = "127.0.0.1"
port = 18080

[[providers]]
provider_instance_name = "openai-mock-1"
provider_type = "cloud_api"
provider_driver = "openai"
base_path = "/v1"
health = "available"
quota_state = "normal"

[[providers.models]]
provider_model_id = "gpt-5-mini"
exact_model = "gpt-5-mini@openai-mock-1"
api_types = ["llm.chat", "llm.completion", "embedding.text", "image.txt2img"]
logical_mounts = ["llm.gpt5", "llm.chat", "embedding.text", "image.txt2img"]
features = ["json_output", "tool_calling", "web_search", "vision", "streaming"]
max_context_tokens = 128000
quality_score = 0.90
latency_ms = 20
cost_per_1k_input_tokens = 0.0
cost_per_1k_output_tokens = 0.0

[[scenarios]]
name = "success"
status = 200
latency_ms = 20
usage_input_tokens = 10
usage_output_tokens = 5

[[scenarios]]
name = "rate_limit"
status = 429
provider_code = "mock/rate_limit"

[[scenarios]]
name = "stream_success"
status = 200
stream_chunks = ["hello", " ", "world"]
usage_input_tokens = 10
usage_output_tokens = 3
```

配置要求：

- Provider settings 中的 `base_url` 指向 Mock Provider。
- Mock Provider 返回的 inventory、health、usage 和错误码必须可由配置控制。
- 同一个 scenario 在 Rust Mock 和 TS Mock 中语义一致。
- scenario 触发方式必须稳定，推荐通过 `payload.options.mock_behavior.scenario` 指定。

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
