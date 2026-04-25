# AICC API 设计

版本：`v0.1-draft`
更新基线：`2026-04-25`
配套文档：

- `doc/aicc/aicc_api建议.md`
- `doc/aicc/aicc 逻辑模型目录.md`
- `doc/aicc/aicc_router.md`
- `doc/aicc/update_aicc_settings_via_system_config.md`

本文定义 AICC 面向调用方、Provider Adapter、Router、Control Panel 和 Agent Runtime 的标准 API 设计。目标是覆盖 `aicc 逻辑模型目录.md` 中规划的所有已知 AI API 调用类型，同时保持和当前 AICC 实现的兼容关系。

---

## 1. 设计原则

### 1.1 核心入口保持 BuckyOS kRPC

AICC 是 BuckyOS 系统服务，核心调用入口保持：

```text
POST /kapi/aicc
```

核心 kRPC 方法：

| method | 语义 |
|---|---|
| `complete` | 发起一次 AI 计算。同步任务直接返回结果；异步任务返回 task id。 |
| `cancel` | best-effort 取消 `complete` 返回的 task。 |
| `reload_settings` / `service.reload_settings` | 从 `services/aicc/settings` 重新加载 Provider 配置。 |

不为核心调用另建 `/v1/invoke`、`/v1/jobs`、`/v1/objects`。如果未来需要 OpenAI-compatible 或 REST-compatible API，应放在 Gateway Adapter / SDK Facade 层，把请求转换成 AICC kRPC。

### 1.2 `api_type` 决定 schema

`api_type` 是 canonical schema discriminator。Provider 不能自定义 `api_type`，只能声明自己支持标准集合中的哪些类型。

`Capability` 保持为当前 v0 兼容入口；`api_type` 作为更精确的 schema 字段渐进引入：

```rust
pub struct CompleteRequestV1 {
    pub capability: Capability,
    pub api_type: Option<ApiType>,
    pub model: ModelSpec,
    pub requirements: Requirements,
    pub payload: AiPayload,
    pub policy: Option<RoutePolicy>,
    pub idempotency_key: Option<String>,
    pub task_options: Option<CompleteTaskOptions>,
}
```

兼容规则：

1. 旧调用方只传 `capability`，AICC 根据 capability 和 payload 推导 `api_type`。
2. 新调用方传 `api_type`，Router 和 Provider Adapter 必须按 `api_type` 解释 request / response。
3. fallback 不得改变显式 `api_type`。
4. `Capability` 不表达细 schema，只表达粗能力和权限边界。

### 1.3 数据面复用 BuckyOS ResourceRef / NamedObject

AICC 不引入私有 Object Store。非结构化数据通过当前 `ResourceRef` 传递：

```rust
pub enum ResourceRef {
    Url { url: String, mime_hint: Option<String> },
    Base64 { mime: String, data_base64: String },
    NamedObject { obj_id: ObjId },
}
```

v1 建议增加 metadata 包装：

```rust
pub struct ResourceDescriptor {
    pub resource: ResourceRef,
    pub media_type: Option<String>,
    pub modality: Option<ResourceModality>,
    pub size_bytes: Option<u64>,
    pub digest: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub labels: Option<serde_json::Value>,
}
```

Router、policy、日志层只看 descriptor 和 metadata，不读取 bytes。Provider Adapter 只能在最后执行阶段读取 `ResourceRef`。

### 1.4 任务生命周期复用 task-manager

AICC 不定义私有 Job API。长任务使用 `task-manager`：

| AICC response | task-manager 状态 | 说明 |
|---|---|---|
| `status=succeeded` | `Completed` | 同步完成，`result` 非空。 |
| `status=running` | `Pending` / `Running` | 异步任务已排队或运行。 |
| `status=failed` | `Failed` | 启动阶段失败或无法提交 Provider。 |

视频生成、音乐生成、大批量 embedding、长文件转录等默认走异步任务。

---

## 2. 顶层协议

### 2.1 `complete` request

```json
{
  "method": "complete",
  "params": {
    "capability": "llm_router",
    "api_type": "llm.chat",
    "model": {
      "alias": "llm.plan",
      "provider_model_hint": null
    },
    "requirements": {
      "must_features": ["json_output"],
      "max_latency_ms": 3000,
      "max_cost_usd": 0.05,
      "resp_format": "json",
      "extra": {}
    },
    "payload": {
      "text": null,
      "messages": [],
      "tool_specs": [],
      "resources": [],
      "resource_descriptors": [],
      "input_json": {},
      "options": {}
    },
    "policy": {
      "profile": "balanced",
      "allow_fallback": true,
      "runtime_failover": true,
      "explain": false
    },
    "idempotency_key": "client-req-001",
    "task_options": {
      "parent_id": null
    }
  },
  "sys": [1001, "<session_token>", "trace-aicc-001"]
}
```

兼容说明：

1. 当前实现中的 `resp_foramt` 拼写保持兼容，新文档使用 `resp_format`。
2. 当前实现没有顶层 `api_type` 和 `policy` 时，可临时放到 `payload.options.api_type` 与 `requirements.extra`。
3. 当前实现没有 `resource_descriptors` 时，仍使用 `payload.resources`。

### 2.2 `complete` response

同步成功：

```json
{
  "task_id": "aicc-001",
  "status": "succeeded",
  "result": {
    "text": "answer",
    "tool_calls": [],
    "artifacts": [],
    "usage": {
      "input_tokens": 100,
      "output_tokens": 30,
      "total_tokens": 130
    },
    "cost": {
      "amount": 0.002,
      "currency": "USD"
    },
    "finish_reason": "stop",
    "provider_task_ref": null,
    "extra": {
      "api_type": "llm.chat",
      "route_trace": {}
    }
  },
  "event_ref": "task://aicc-001/events"
}
```

异步启动：

```json
{
  "task_id": "aicc-002",
  "status": "running",
  "result": null,
  "event_ref": "task://aicc-002/events"
}
```

启动失败：

```json
{
  "task_id": "aicc-003",
  "status": "failed",
  "result": null,
  "event_ref": "task://aicc-003/events"
}
```

错误细节写入 task event / task data。kRPC error 只用于 transport、鉴权、反序列化、服务异常等系统错误。

### 2.3 `cancel`

Request：

```json
{
  "method": "cancel",
  "params": {
    "task_id": "aicc-002"
  },
  "sys": [1002, "<session_token>", "trace-aicc-cancel"]
}
```

Response：

```json
{
  "task_id": "aicc-002",
  "accepted": true
}
```

语义：

1. `accepted=true` 表示 AICC 已接受取消请求，并尝试通知 Provider / task-manager。
2. `accepted=false` 表示 task binding 不存在、任务已结束或 Provider 不支持取消。
3. 跨 tenant cancel 必须拒绝。

---

## 3. 通用数据结构

### 3.1 ResourceDescriptor

```json
{
  "resource": {
    "kind": "named_object",
    "obj_id": "chunk:..."
  },
  "media_type": "image/png",
  "modality": "image",
  "size_bytes": 2453312,
  "digest": "sha256:...",
  "metadata": {
    "width": 1024,
    "height": 768,
    "filename": "input.png"
  },
  "labels": {
    "pii": false,
    "retention_class": "session"
  }
}
```

`modality` 标准值：

| modality | 说明 |
|---|---|
| `text` | 文本、Markdown、字幕、OCR 文本。 |
| `image` | 图片。 |
| `mask` | 图像 mask。 |
| `audio` | 音频。 |
| `video` | 视频。 |
| `document` | PDF、Office 文档、网页归档。 |
| `tensor` | embedding / tensor 矩阵。 |
| `archive` | 压缩包。 |
| `unknown` | 未知。 |

### 3.2 Content Part

LLM 多模态消息建议使用 content part：

```json
{
  "role": "user",
  "content": [
    { "type": "text", "text": "请解释这张图。" },
    {
      "type": "resource",
      "resource": {
        "resource": { "kind": "named_object", "obj_id": "chunk:image" },
        "media_type": "image/png",
        "modality": "image"
      }
    }
  ]
}
```

v0 兼容：

1. 当前 `AiMessage.content` 是 string；纯文本继续走该字段。
2. 多模态内容可临时放在 `payload.input_json.messages_v2`。
3. `payload.resources` 继续用于没有 content part 支持的调用方。

### 3.3 Generation Parameters

```json
{
  "temperature": 0.7,
  "top_p": 1.0,
  "max_output_tokens": 2048,
  "seed": 12345,
  "stop": ["</final>"],
  "response_mime_type": "application/json"
}
```

### 3.4 Usage

```json
{
  "input_tokens": 1024,
  "output_tokens": 512,
  "total_tokens": 1536,
  "cached_input_tokens": 300,
  "reasoning_tokens": 128,
  "audio_seconds": 12.4,
  "video_seconds": 8,
  "image_count": 1,
  "request_units": 1,
  "cost_usd": 0.0123
}
```

当前 `AiUsage` 只包含 token 字段；扩展字段可以先进入 `AiResponseSummary.extra.usage_ext`。

### 3.5 Bounding Box

```json
{
  "format": "xywh",
  "unit": "px",
  "x": 120,
  "y": 80,
  "width": 300,
  "height": 600
}
```

`unit` 可为 `px` 或 `relative`。`relative` 范围为 `[0, 1]`。

### 3.6 Mask

```json
{
  "format": "rle",
  "size": [1024, 1024],
  "counts": "..."
}
```

支持：

| format | 说明 |
|---|---|
| `rle` | COCO-style run length encoding。 |
| `polygon` | 多边形点集。 |
| `bitmap_resource` | 通过 `ResourceDescriptor` 引用 bitmap mask。 |

---

## 4. API Type 总览

| api_type | 默认逻辑目录 | Capability | 默认任务模式 | 当前实现状态 |
|---|---|---|---|---|
| `llm.chat` | `llm.chat` / `llm.*` | `llm_router` | sync 或 async | 已部分实现。 |
| `llm.completion` | `llm.completion` | `llm_router` | sync | 内部枚举已存在。 |
| `embedding.text` | `embedding.text` | v1 新增 `embedding` 或显式 api_type | sync 或 async | 待实现；内部有 `embedding` 粗枚举。 |
| `embedding.multimodal` | `embedding.multimodal` | v1 新增 `embedding` 或显式 api_type | sync 或 async | 待实现。 |
| `rerank` | `rerank.general` | v1 新增 `rerank` 或显式 api_type | sync | 待实现。 |
| `image.txt2img` | `image.txt2img` | `text2_image` | sync 或 async | 当前内部名 `image.txt2image`，需兼容映射。 |
| `image.img2img` | `image.img2img` | v1 新增 `image2_image` 或显式 api_type | sync 或 async | 当前内部名 `image.img2image`。 |
| `image.inpaint` | `image.inpaint` | 显式 api_type | sync 或 async | 待实现。 |
| `image.upscale` | `image.upscale` | 显式 api_type | sync 或 async | 待实现。 |
| `image.bg_remove` | `image.bg_remove` | 显式 api_type | sync | 待实现。 |
| `vision.ocr` | `image.ocr` | `image2_text` 或显式 api_type | sync 或 async | 待实现。 |
| `vision.caption` | `image.caption` | `image2_text` 或显式 api_type | sync | 待实现。 |
| `vision.detect` | `image.detect` | `image2_text` 或显式 api_type | sync | 待实现。 |
| `vision.segment` | `image.segment` | 显式 api_type | sync | 待实现。 |
| `audio.tts` | `audio.tts` | `text2_voice` | sync 或 async | 待实现。 |
| `audio.asr` | `audio.asr` | `voice2_text` | sync 或 async | 待实现。 |
| `audio.music` | `audio.music` | 显式 api_type | async | 待实现。 |
| `audio.enhance` | `audio.enhance` | 显式 api_type | sync 或 async | 待实现。 |
| `video.txt2video` | `video.txt2video` | `text2_video` | async | 待实现。 |
| `video.img2video` | `video.img2video` | 显式 api_type | async | 待实现。 |
| `video.video2video` | `video.video2video` | 显式 api_type | async | 待实现。 |
| `video.extend` | `video.extend` | 显式 api_type | async | 待实现。 |
| `video.upscale` | `video.upscale` | 显式 api_type | async | 待实现。 |
| `agent.computer_use` | `agent_runtime.computer_use` | v1 新增 `agent_runtime` 或由 Agent Runtime 代理 | session async | 占位，不建议直接进入 AICC v0。 |

命名兼容：

1. 逻辑模型目录使用 `image.txt2img` / `image.img2img`。
2. 当前 Rust 内部已有 `image.txt2image` / `image.img2image`。
3. v1 实现应同时接受两组名字，canonical 输出优先使用逻辑模型目录中的 `txt2img` / `img2img`。

---

## 5. LLM API

### 5.1 `llm.chat`

用途：通用对话、Agent plan/code/reason/summary、VQA、多模态聊天、工具调用。

Request canonical body 放在 `payload.input_json`：

```json
{
  "messages": [
    {
      "role": "developer",
      "content": [
        { "type": "text", "text": "You are a coding assistant." }
      ]
    },
    {
      "role": "user",
      "content": [
        { "type": "text", "text": "解释这张图。" },
        {
          "type": "resource",
          "resource": {
            "resource": { "kind": "named_object", "obj_id": "chunk:image" },
            "media_type": "image/png",
            "modality": "image"
          }
        }
      ]
    }
  ],
  "tools": [
    {
      "type": "function",
      "name": "get_weather",
      "description": "Get weather by city.",
      "args_json_schema": {
        "type": "object",
        "properties": {
          "city": { "type": "string" }
        },
        "required": ["city"],
        "additionalProperties": false
      }
    }
  ],
  "response_format": {
    "type": "json_schema",
    "json_schema": {
      "name": "answer",
      "schema": {
        "type": "object",
        "properties": {
          "summary": { "type": "string" }
        },
        "required": ["summary"],
        "additionalProperties": false
      }
    }
  }
}
```

Response mapping：

| 输出 | AICC 字段 |
|---|---|
| assistant text | `AiResponseSummary.text` |
| tool calls | `AiResponseSummary.tool_calls` |
| finish reason | `AiResponseSummary.finish_reason` |
| provider 原生响应摘要 | `AiResponseSummary.extra.provider_io`，必须脱敏。 |
| route trace | `AiResponseSummary.extra.route_trace` 或 task data。 |

Fallback：

1. `llm.plan`、`llm.code`、`llm.summary` 可 parent fallback 到 `llm`。
2. `llm.reason` 默认 disabled 或 strict，避免静默降级到无 reasoning 能力模型。
3. `llm.vision` 必须硬过滤 `vision=true`。

### 5.2 `llm.completion`

用途：legacy completion。新调用方应使用 `llm.chat`。

Request：

```json
{
  "prompt": "Complete this text: The future of AI is",
  "suffix": null
}
```

Response：

```json
{
  "text": "...",
  "finish_reason": "stop"
}
```

Response mapping：`text` 写入 `AiResponseSummary.text`。

---

## 6. Embedding API

### 6.1 `embedding.text`

用途：文本、代码、文档 chunk embedding。

Request：

```json
{
  "items": [
    { "type": "text", "text": "hello world", "id": "item-1" },
    {
      "type": "resource",
      "id": "doc-1",
      "resource": {
        "resource": { "kind": "named_object", "obj_id": "chunk:doc" },
        "media_type": "text/markdown",
        "modality": "text"
      }
    }
  ],
  "chunking": {
    "strategy": "auto",
    "max_tokens": 800,
    "overlap_tokens": 80
  },
  "embedding_space_id": null,
  "dimensions": 1024,
  "normalize": true
}
```

小批量 response：

```json
{
  "data": [
    {
      "index": 0,
      "id": "item-1",
      "embedding": [0.0123, -0.0456],
      "embedding_space_id": "bge-m3:1024:cosine:normalized:v1"
    }
  ]
}
```

大批量 response：

```json
{
  "data_resource": {
    "resource": { "kind": "named_object", "obj_id": "chunk:embeddings" },
    "media_type": "application/vnd.buckyos.aicc.embeddings+parquet",
    "modality": "tensor",
    "metadata": {
      "rows": 1000000,
      "dimensions": 1024,
      "embedding_space_id": "bge-m3:1024:cosine:normalized:v1"
    }
  }
}
```

Response mapping：

1. 小批量数据放 `AiResponseSummary.extra.embedding`。
2. 大批量数据生成 `AiArtifact`，resource 使用 `NamedObject`。
3. `embedding_space_id` 必须进入结果 metadata。

Fallback：

1. 默认 strict。
2. 如果 request 指定 `embedding_space_id`，fallback 后的模型必须产出相同 space。
3. 已存在向量索引查询时，禁止 fallback 到不同 space。

### 6.2 `embedding.multimodal`

用途：CLIP / SigLIP 类跨模态 embedding。

Request：

```json
{
  "items": [
    {
      "id": "pair-1",
      "text": "a red car",
      "image": {
        "resource": { "kind": "named_object", "obj_id": "chunk:image" },
        "media_type": "image/png",
        "modality": "image"
      }
    }
  ],
  "dimensions": 1408,
  "normalize": true
}
```

Response 与 `embedding.text` 相同。fallback 同样必须保证 embedding space 兼容。

---

## 7. Rerank API

### 7.1 `rerank`

用途：Cross-encoder / late interaction 文档重排序。

Request：

```json
{
  "query": "What is the refund policy?",
  "documents": [
    {
      "id": "doc-1",
      "text": "Refunds are available within 30 days.",
      "metadata": { "source": "policy.md" }
    },
    {
      "id": "doc-2",
      "resource": {
        "resource": { "kind": "named_object", "obj_id": "chunk:doc-2" },
        "media_type": "text/plain",
        "modality": "text"
      }
    }
  ],
  "top_n": 5,
  "return_documents": false
}
```

Response：

```json
{
  "results": [
    {
      "index": 0,
      "id": "doc-1",
      "score": 0.98
    }
  ]
}
```

Response mapping：结果放 `AiResponseSummary.extra.rerank`。

Fallback：默认 strict。不同 reranker 分数不可直接比较，fallback 只允许在同一任务内重跑，不允许和旧分数混排。

---

## 8. Image API

### 8.1 `image.txt2img`

Request：

```json
{
  "prompt": "A precise product photo of a matte black desk lamp.",
  "negative_prompt": "blurry, low quality",
  "n": 1,
  "size": "1024x1024",
  "aspect_ratio": "1:1",
  "quality": "high",
  "seed": 12345,
  "output_media_type": "image/png"
}
```

Response：

```json
{
  "images": [
    {
      "resource": { "kind": "named_object", "obj_id": "chunk:generated-image" },
      "media_type": "image/png",
      "modality": "image",
      "metadata": {
        "width": 1024,
        "height": 1024
      }
    }
  ]
}
```

Response mapping：每张图片生成 `AiArtifact`。

### 8.2 `image.img2img`

Request：

```json
{
  "images": [
    {
      "resource": { "kind": "named_object", "obj_id": "chunk:source-image" },
      "media_type": "image/png",
      "modality": "image"
    }
  ],
  "prompt": "Change the background to a sunny beach.",
  "strength": 0.6,
  "output_media_type": "image/png"
}
```

Response：同 `image.txt2img`。

### 8.3 `image.inpaint`

Request：

```json
{
  "image": {
    "resource": { "kind": "named_object", "obj_id": "chunk:image" },
    "media_type": "image/png",
    "modality": "image"
  },
  "mask": {
    "resource": { "kind": "named_object", "obj_id": "chunk:mask" },
    "media_type": "image/png",
    "modality": "mask"
  },
  "prompt": "Add a vase of flowers on the table.",
  "mask_semantics": "white_area_is_edit_area",
  "output_media_type": "image/png"
}
```

Response：同 `image.txt2img`。

### 8.4 `image.upscale`

Request：

```json
{
  "image": {
    "resource": { "kind": "named_object", "obj_id": "chunk:image" },
    "media_type": "image/jpeg",
    "modality": "image"
  },
  "scale": 4,
  "target_width": 4096,
  "target_height": 4096,
  "preserve_faces": true,
  "output_media_type": "image/png"
}
```

Response：

```json
{
  "image": {
    "resource": { "kind": "named_object", "obj_id": "chunk:upscaled" },
    "media_type": "image/png",
    "modality": "image"
  }
}
```

### 8.5 `image.bg_remove`

Request：

```json
{
  "image": {
    "resource": { "kind": "named_object", "obj_id": "chunk:image" },
    "media_type": "image/jpeg",
    "modality": "image"
  },
  "output": "rgba_image",
  "output_media_type": "image/png"
}
```

Response：

```json
{
  "image": {
    "resource": { "kind": "named_object", "obj_id": "chunk:rgba" },
    "media_type": "image/png",
    "modality": "image",
    "metadata": {
      "alpha": true
    }
  }
}
```

Image fallback：

1. `txt2img` 可 parent fallback，但必须保持 `api_type=image.txt2img`。
2. `inpaint` fallback 必须保持 mask 语义一致。
3. `upscale` fallback 必须满足目标分辨率和人脸保护等硬约束。

---

## 9. Vision API

Vision API 用于结构化图像理解。自由文本 VQA 使用 `llm.chat`，并在 message content 中传 image resource。

### 9.1 `vision.ocr`

Request：

```json
{
  "document": {
    "resource": { "kind": "named_object", "obj_id": "chunk:page" },
    "media_type": "image/png",
    "modality": "image",
    "metadata": {
      "width": 2480,
      "height": 3508
    }
  },
  "level": "word",
  "language_hints": ["zh", "en"],
  "return_layout": true,
  "return_artifacts": ["plain_text", "alto_json"]
}
```

Response：

```json
{
  "pages": [
    {
      "page_index": 0,
      "width": 2480,
      "height": 3508,
      "blocks": [
        {
          "type": "text",
          "bbox": {
            "format": "xywh",
            "unit": "px",
            "x": 100,
            "y": 200,
            "width": 500,
            "height": 120
          },
          "lines": [
            {
              "text": "Example text",
              "confidence": 0.98
            }
          ]
        }
      ]
    }
  ],
  "artifacts": {
    "plain_text": {
      "resource": { "kind": "named_object", "obj_id": "chunk:ocr-text" },
      "media_type": "text/plain",
      "modality": "text"
    }
  }
}
```

Response mapping：

1. 纯文本摘要写 `AiResponseSummary.text`。
2. 结构化 OCR 写 `AiResponseSummary.extra.ocr`。
3. OCR artifact 写 `AiResponseSummary.artifacts`。

### 9.2 `vision.caption`

Request：

```json
{
  "image": {
    "resource": { "kind": "named_object", "obj_id": "chunk:image" },
    "media_type": "image/jpeg",
    "modality": "image"
  },
  "style": "short",
  "language": "zh-CN",
  "max_captions": 3
}
```

Response：

```json
{
  "captions": [
    {
      "text": "一盏黑色台灯放在木质桌面上。",
      "confidence": 0.93
    }
  ]
}
```

`captions[0].text` 可同步写入 `AiResponseSummary.text`。

### 9.3 `vision.detect`

Request：

```json
{
  "image": {
    "resource": { "kind": "named_object", "obj_id": "chunk:image" },
    "media_type": "image/jpeg",
    "modality": "image"
  },
  "classes": ["person", "car"],
  "score_threshold": 0.3,
  "bbox_format": "xywh",
  "bbox_unit": "px"
}
```

Response：

```json
{
  "detections": [
    {
      "label": "person",
      "class_id": "person",
      "score": 0.97,
      "bbox": {
        "format": "xywh",
        "unit": "px",
        "x": 120,
        "y": 80,
        "width": 300,
        "height": 600
      }
    }
  ]
}
```

Response mapping：写入 `AiResponseSummary.extra.detections`。

### 9.4 `vision.segment`

Request：

```json
{
  "image": {
    "resource": { "kind": "named_object", "obj_id": "chunk:image" },
    "media_type": "image/png",
    "modality": "image"
  },
  "prompt": {
    "type": "box",
    "bbox": {
      "format": "xywh",
      "unit": "px",
      "x": 120,
      "y": 80,
      "width": 300,
      "height": 600
    }
  },
  "mask_format": "rle",
  "return_bitmap_mask": false
}
```

Response：

```json
{
  "masks": [
    {
      "id": "mask-1",
      "score": 0.95,
      "bbox": {
        "format": "xywh",
        "unit": "px",
        "x": 120,
        "y": 80,
        "width": 300,
        "height": 600
      },
      "mask": {
        "format": "rle",
        "size": [1024, 1024],
        "counts": "..."
      }
    }
  ]
}
```

Response mapping：结构化 mask 写 `extra.segment`; bitmap mask 写 `artifacts`。

---

## 10. Audio API

### 10.1 `audio.tts`

Request：

```json
{
  "text": "你好，欢迎使用 AICC。",
  "voice": {
    "voice_id": "voice_zh_female_warm_001",
    "language": "zh-CN",
    "gender": "female",
    "style": "warm",
    "speaker_similarity_required": false
  },
  "speed": 1.0,
  "response_media_type": "audio/mpeg",
  "sample_rate": 44100
}
```

Response：

```json
{
  "audio": {
    "resource": { "kind": "named_object", "obj_id": "chunk:tts-audio" },
    "media_type": "audio/mpeg",
    "modality": "audio",
    "metadata": {
      "duration_seconds": 3.2,
      "sample_rate": 44100,
      "channels": 2
    }
  }
}
```

Fallback：

1. 如果指定 `voice_id` 且 `speaker_similarity_required=true`，禁止跨 Provider fallback。
2. 如果只指定 language / gender / style，可在满足 voice contract 的 Provider 内 fallback。

### 10.2 `audio.asr`

Request：

```json
{
  "audio": {
    "resource": { "kind": "named_object", "obj_id": "chunk:meeting-audio" },
    "media_type": "audio/mpeg",
    "modality": "audio",
    "metadata": {
      "duration_seconds": 312.4
    }
  },
  "language": "zh-CN",
  "timestamps": "segment",
  "diarization": true,
  "output_formats": ["json", "vtt", "srt"]
}
```

Response：

```json
{
  "text": "大家好，欢迎来到今天的会议。",
  "segments": [
    {
      "id": "seg-1",
      "start_seconds": 0.0,
      "end_seconds": 2.4,
      "text": "大家好，欢迎来到今天的会议。",
      "speaker": "SPEAKER_0",
      "confidence": 0.94
    }
  ],
  "artifacts": {
    "vtt": {
      "resource": { "kind": "named_object", "obj_id": "chunk:subtitle-vtt" },
      "media_type": "text/vtt",
      "modality": "text"
    }
  }
}
```

Response mapping：

1. transcript 写 `AiResponseSummary.text`。
2. segments 写 `AiResponseSummary.extra.asr.segments`。
3. subtitles 写 `AiResponseSummary.artifacts`。

### 10.3 `audio.music`

Request：

```json
{
  "prompt": "A 30-second cheerful acoustic folk song with guitar.",
  "duration_seconds": 30,
  "instrumental": false,
  "lyrics": null,
  "output_media_type": "audio/mpeg",
  "seed": 12345
}
```

Response：

```json
{
  "audio": {
    "resource": { "kind": "named_object", "obj_id": "chunk:music" },
    "media_type": "audio/mpeg",
    "modality": "audio"
  },
  "structure": {
    "lyrics": "...",
    "sections": [
      { "name": "intro", "start_seconds": 0, "end_seconds": 8 }
    ]
  }
}
```

默认异步。

### 10.4 `audio.enhance`

Request：

```json
{
  "audio": {
    "resource": { "kind": "named_object", "obj_id": "chunk:noisy-audio" },
    "media_type": "audio/wav",
    "modality": "audio"
  },
  "task": "denoise",
  "strength": 0.8,
  "return_stems": false
}
```

Response：

```json
{
  "audio": {
    "resource": { "kind": "named_object", "obj_id": "chunk:enhanced-audio" },
    "media_type": "audio/wav",
    "modality": "audio"
  },
  "stems": []
}
```

---

## 11. Video API

视频生成和编辑默认异步，`complete` 返回 task id，最终结果写 task data / event。

### 11.1 `video.txt2video`

Request：

```json
{
  "prompt": "A cinematic tracking shot through a lantern-lit market at dusk.",
  "duration_seconds": 8,
  "aspect_ratio": "16:9",
  "resolution": "720p",
  "fps": 24,
  "generate_audio": true,
  "seed": 12345
}
```

Final result：

```json
{
  "video": {
    "resource": { "kind": "named_object", "obj_id": "chunk:generated-video" },
    "media_type": "video/mp4",
    "modality": "video",
    "metadata": {
      "duration_seconds": 8,
      "width": 1280,
      "height": 720,
      "fps": 24
    }
  }
}
```

### 11.2 `video.img2video`

Request：

```json
{
  "image": {
    "resource": { "kind": "named_object", "obj_id": "chunk:start-frame" },
    "media_type": "image/png",
    "modality": "image"
  },
  "prompt": "Animate the scene with slow camera movement.",
  "duration_seconds": 8,
  "aspect_ratio": "16:9",
  "resolution": "720p"
}
```

Response：同 `video.txt2video`。

### 11.3 `video.video2video`

Request：

```json
{
  "video": {
    "resource": { "kind": "named_object", "obj_id": "chunk:source-video" },
    "media_type": "video/mp4",
    "modality": "video"
  },
  "prompt": "Shift the color palette to teal and warm backlight.",
  "preserve_motion": true,
  "time_range": {
    "start_seconds": 0,
    "end_seconds": 8
  }
}
```

Response：同 `video.txt2video`。

### 11.4 `video.extend`

Request：

```json
{
  "video": {
    "resource": { "kind": "named_object", "obj_id": "chunk:previous-video" },
    "media_type": "video/mp4",
    "modality": "video"
  },
  "prompt": "Continue the shot as the camera rises over the rooftops.",
  "duration_seconds": 7,
  "resolution": "720p"
}
```

Response：同 `video.txt2video`。

### 11.5 `video.upscale`

Request：

```json
{
  "video": {
    "resource": { "kind": "named_object", "obj_id": "chunk:source-video" },
    "media_type": "video/mp4",
    "modality": "video"
  },
  "target_resolution": "4k",
  "fps": 24,
  "denoise": true,
  "sharpen": 0.3
}
```

Response：同 `video.txt2video`。

Video fallback：

1. 只能在同 `api_type` 内 fallback。
2. Provider 一旦返回 Started / Queued，AICC 不再跨 Provider 重试。
3. `extend` 必须保持源视频和 provider operation 的状态一致，默认 strict。

---

## 12. Agent Runtime API

### 12.1 `agent.computer_use`

`agent.computer_use` 是 `aicc 逻辑模型目录.md` 中的占位方向。它依赖外部环境状态，不建议作为 AICC v0 普通模型调用直接开放。推荐架构：

```text
Agent Runtime / OpenDAN
  -> 管理 environment/session/sandbox
  -> 调用 AICC complete(api_type=agent.computer_use)
  -> 执行动作并回传下一帧 observation
```

Request：

```json
{
  "task": "Click the login button and enter the username.",
  "environment": {
    "environment_id": "sandbox-123",
    "session_id": "agent-session-001",
    "screenshot": {
      "resource": { "kind": "named_object", "obj_id": "chunk:screenshot" },
      "media_type": "image/png",
      "modality": "image",
      "metadata": {
        "width": 1280,
        "height": 720
      }
    },
    "viewport": {
      "width": 1280,
      "height": 720
    }
  },
  "allowed_actions": [
    "screenshot",
    "left_click",
    "right_click",
    "type",
    "key",
    "scroll",
    "wait"
  ]
}
```

Response：

```json
{
  "actions": [
    {
      "type": "left_click",
      "x": 640,
      "y": 520
    },
    {
      "type": "type",
      "text": "alice@example.com"
    }
  ],
  "requires_next_observation": true
}
```

Fallback：

1. 默认 strict 或 sticky_session。
2. 不允许静默更换 `environment_id`。
3. 是否可 fallback 由 Agent Runtime 的 session 策略决定，AICC Router 不单独判断 sandbox 可迁移性。

---

## 13. Provider Inventory 要求

每个 Provider instance 必须声明自己支持的 api_type：

```json
{
  "provider_instance_name": "openai_primary",
  "provider_type": "cloud_api",
  "provider_driver": "openai",
  "models": [
    {
      "provider_model_id": "gpt-5.5",
      "exact_model": "gpt-5.5@openai_primary",
      "api_types": ["llm.chat"],
      "logical_mounts": ["llm.gpt5", "llm.plan", "llm.code", "llm.vision"],
      "capabilities": {
        "tool_call": true,
        "json_schema": true,
        "vision": true,
        "max_context_tokens": 400000
      },
      "attributes": {
        "provider_type": "cloud_api",
        "privacy": "public_cloud",
        "quality_score": 0.95,
        "latency_class": "normal",
        "cost_class": "high"
      },
      "pricing": {},
      "health": {
        "status": "available",
        "quota_state": "normal"
      }
    }
  ]
}
```

约束：

1. `exact_model` 必须是 `<provider_model_id>@<provider_instance_name>`。
2. `api_types` 是 Router 硬过滤条件。
3. `logical_mounts` 只负责候选展开，不决定 schema。
4. `provider_type` 的可信来源应是 system-config 或 admin override，不能只信 Provider 自声明。

---

## 14. 逻辑目录映射

### 14.1 一级目录

| 一级目录 | 默认 api_type | fallback |
|---|---|---|
| `llm` | `llm.chat` | parent |
| `embedding` | `embedding.text` / `embedding.multimodal` | strict |
| `rerank` | `rerank` | strict |
| `image` | `image.*` / `vision.*` | same api_type parent |
| `audio` | `audio.*` | strict 或 voice contract |
| `video` | `video.*` | same api_type parent |
| `agent_runtime` | `agent.*` | sticky_session |

### 14.2 调用方选择模型

调用方应使用逻辑路径：

```json
{
  "api_type": "vision.ocr",
  "model": {
    "alias": "image.ocr"
  }
}
```

调试或强制指定 Provider 时使用精确模型：

```json
{
  "api_type": "llm.chat",
  "model": {
    "alias": "claude-sonnet-4.6@anthropic"
  }
}
```

精确模型默认不 fallback，除非 policy 明确允许。

---

## 15. 错误模型

标准错误分层：

| 层级 | 表达方式 |
|---|---|
| kRPC transport / auth / parse | `RPCErrors` |
| AICC 启动失败 | `CompleteResponse.status=failed` + task event |
| Provider 执行失败 | task-manager `Failed` + task event |
| fallback / failover | route trace + task event |

建议错误码：

| code | 说明 |
|---|---|
| `invalid_request` | 请求格式或字段非法。 |
| `invalid_api_type` | 未知或不支持的 api_type。 |
| `schema_validation_failed` | request 未通过 api_type schema 校验。 |
| `resource_invalid` | 资源不存在、无权限、格式不支持或过大。 |
| `no_provider_available` | 无 Provider 支持 capability。 |
| `no_candidate_model` | route 过滤后无候选。 |
| `fallback_not_allowed` | fallback 被 policy 或 api_type 禁止。 |
| `provider_start_failed` | Provider 启动或提交失败。 |
| `provider_error` | Provider 原生错误。 |
| `timeout` | 超时。 |
| `budget_exceeded` | 成本或配额限制。 |
| `policy_denied` | 被 system/user/session policy 拒绝。 |
| `internal_error` | 内部错误。 |

---

## 16. 落地顺序

### M0：保持当前可用协议

1. `/kapi/aicc complete/cancel` 作为稳定入口。
2. `Capability + ModelSpec.alias + Requirements + AiPayload` 不破坏。
3. `ResourceRef` 保持当前三种形态。

### M1：ApiType 显式化

1. `CompleteRequest` 增加 `api_type: Option<ApiType>`。
2. `payload.options.api_type` 作为过渡兼容。
3. `ProviderInventory.models[].api_types` 作为硬过滤。
4. `image.txt2img` 与 `image.txt2image` 做兼容 alias。

### M2：ResourceDescriptor

1. 增加 `payload.resource_descriptors`。
2. Provider Adapter 只在最后一跳读取资源。
3. Router 只读取 metadata / labels。

### M3：逐类实现 schema

优先级建议：

1. `llm.chat` 多模态和 tool schema。
2. `image.txt2img` / `image.img2img`。
3. `audio.asr` / `audio.tts`。
4. `embedding.text` / `rerank`。
5. `vision.ocr` / `vision.caption`。
6. `video.*` 异步任务。
7. `agent.computer_use` 由 Agent Runtime 牵头接入。

每新增一个 api_type，必须同步：

1. API schema 文档。
2. `ApiType` 枚举。
3. Provider inventory 声明。
4. Router fallback / policy 规则。
5. Provider Adapter 映射。
6. task-manager 状态和事件。
7. 单元测试或 DV test。

