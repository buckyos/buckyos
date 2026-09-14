# AICC 用户接口冻结文档

状态：**Frozen / Beta 2.2**

冻结日期：2026-09-14

目标读者：应用开发者、Agent、AICC 管理 UI 与运维工具作者。

本文说明如何使用当前 AICC。字段级唯一真相源是
`src/kernel/buckyos-api/src/aicc_client.rs`；本文冻结调用方式、方法集合和跨方法语义。

## 1. 入口与鉴权

AICC 服务名为 `aicc`，服务端口为 `4040`，统一通过 NodeGateway 的
`POST /kapi/aicc` 调用。推荐 Rust 调用方使用 `buckyos_api::AiccClient`；其它语言按 kRPC 信封调用：

```json
{
  "method": "route.resolve",
  "params": {},
  "sys": [1001, "<session_token>", "trace-aicc-001"]
}
```

`sys[0]` 是请求序号，`sys[1]` 是 session token，`sys[2]` 是可选 trace id。身份以 token/RPC context 为准，不能在 `params` 中指定 tenant 或冒充其它用户。

## 2. 选择调用模式

### 2.1 Helper：通常首选

Helper 接受逻辑模型，一次完成路由与推理。目前冻结两个 Helper：

- `helper.llm_chat`
- `helper.text_to_image`

适合希望随 inventory、成本和健康状态自动选择模型的应用。

```json
{
  "method": "helper.llm_chat",
  "params": {
    "logical_model": "llm.plan",
    "requirements": { "tool_call": true, "json_schema": true },
    "messages": [
      { "role": "user", "content": [{ "type": "text", "text": "给出发布计划" }] }
    ],
    "idempotency_key": "release-plan-42"
  },
  "sys": [1001, "<session_token>", "trace-helper-42"]
}
```

### 2.2 Route + typed inference：需要可观察/固定路由时使用

先调用 `route.resolve`：

```json
{
  "method": "route.resolve",
  "params": {
    "api_type": "llm",
    "logical_model": "llm.plan",
    "requirements": { "tool_call": true, "json_schema": true },
    "policy": { "profile": "quality", "runtime_failover": true }
  },
  "sys": [1002, "<session_token>", "trace-route-42"]
}
```

响应给出 `selected_exact_model`、`selected_model_uid`、Provider/adapter/driver/model identity、operation、能力、fallback candidates 和 inventory revision。随后把 `selected_exact_model` 传给 typed method：

```json
{
  "method": "chat.completions.create",
  "params": {
    "exact_model": "gpt-5.1:reasoning-high@openai-primary",
    "messages": [
      { "role": "user", "content": [{ "type": "text", "text": "给出发布计划" }] }
    ],
    "idempotency_key": "release-plan-42"
  },
  "sys": [1003, "<session_token>", "trace-call-42"]
}
```

`logical_model` 是不含 `@` 的用途路径。`exact_model` 必须是
`<provider_model_id>[:<variant>]@<provider_instance_name>`。typed inference 不接受逻辑模型，也不接受用户传入 `provider_options`。

## 3. 推理 Method

| 能力 | kRPC method | Request -> Response |
| --- | --- | --- |
| LLM | `chat.completions.create` | `LlmChatInvokeRequest` -> `LlmChatInvokeResponse` |
| 文生图 | `images.generate` | `TextToImageInvokeRequest` -> `TextToImageInvokeResponse` |
| 文本 embedding | `embedding.text` | `EmbeddingTextRequest` -> `EmbeddingTextResponse` |
| 多模态 embedding | `embedding.multimodal` | `EmbeddingMultimodalRequest` -> `EmbeddingMultimodalResponse` |
| rerank | `rerank` | `RerankRequest` -> `RerankResponse` |
| 图生图 | `image.img2img` | `ImageToImageRequest` -> `ImageToImageResponse` |
| 局部重绘 | `image.inpaint` | `ImageInpaintRequest` -> `ImageInpaintResponse` |
| 图像放大 | `image.upscale` | `ImageUpscaleRequest` -> `ImageUpscaleResponse` |
| 去背景 | `image.bg_remove` | `ImageBackgroundRemoveRequest` -> `ImageBackgroundRemoveResponse` |
| OCR | `vision.ocr` | `VisionOcrRequest` -> `VisionOcrResponse` |
| 图像描述 | `vision.caption` | `VisionCaptionRequest` -> `VisionCaptionResponse` |
| 目标检测 | `vision.detect` | `VisionDetectRequest` -> `VisionDetectResponse` |
| 图像分割 | `vision.segment` | `VisionSegmentRequest` -> `VisionSegmentResponse` |
| TTS | `audio.tts` | `AudioTextToSpeechRequest` -> `AudioTextToSpeechResponse` |
| ASR | `audio.asr` | `AudioSpeechRecognitionRequest` -> `AudioSpeechRecognitionResponse` |
| 音乐生成 | `audio.music` | `AudioMusicRequest` -> `AudioMusicResponse` |
| 音频增强 | `audio.enhance` | `AudioEnhanceRequest` -> `AudioEnhanceResponse` |
| 文生视频 | `video.txt2video` | `VideoTextToVideoRequest` -> `VideoTextToVideoResponse` |
| 图生视频 | `video.img2video` | `VideoImageToVideoRequest` -> `VideoImageToVideoResponse` |
| 视频到视频 | `video.video2video` | `VideoToVideoRequest` -> `VideoToVideoResponse` |
| 视频续写 | `video.extend` | `VideoExtendRequest` -> `VideoExtendResponse` |
| 视频放大 | `video.upscale` | `VideoUpscaleRequest` -> `VideoUpscaleResponse` |
| Computer Use | `agent.computer_use` | `ComputerUseRequest` -> `ComputerUseResponse` |

method 已存在不表示任意模型都支持该能力。最终可用集合以 `models.list`、Provider inventory 和 route 结果为准。

## 4. 通用请求语义

typed request 共同字段：

| 字段 | 规则 |
| --- | --- |
| `exact_model` | 必填；固定一次物理模型与 Provider Instance |
| `trace_id` | 可选业务 trace；kRPC `sys[2]` 仍是传输 trace |
| `execution_mode` | `immediate`（默认）或 `stream`；以所选 operation 支持为准 |
| `idempotency_key` | 建议对有副作用/计费的请求设置；相同 key 必须对应相同请求体 |
| `task_options.parent_id` | 可选父任务 |
| `session_id` | 用于 session 路由连续性和 Provider 状态连续性 |

Helper 另接受 `logical_model`、`requirements`、`disable`、`policy` 和可选
`session_overlay`。请求字段默认拒绝未知字段，调用方应尽早发现拼写错误。

## 5. 消息与资源

LLM 消息使用 `AiMessage { role, content[] }` 的 content-block 模型，不把多模态内容拼进纯文本。支持的内容形态以 `AiContent` 枚举为准，包括 text、resource、tool call/result、thinking 和受控 Provider state。

非文本输入统一使用：

```json
{ "kind": "url", "url": "https://example/image.png", "mime_hint": "image/png" }
```

```json
{ "kind": "base64", "mime": "image/png", "data_base64": "..." }
```

```json
{ "kind": "named_object", "obj_id": "<BuckyOS ObjId>" }
```

URL 会经过服务端策略、大小与 MIME 校验；Named Object 会做租户鉴权。较大输出应返回 `ResourceRef`/artifact，而不是把二进制放入诊断或错误字段。

## 6. 响应与任务

typed response 共同字段为：`task_id`、`status`、业务结果、`usage?`、`cost?`、
`finish_reason?`、`provider_task_ref?`、`route_trace?`、`event_ref?`、`error?`。

`status` 值为 `succeeded | running | failed`：

- `succeeded`：业务结果已经可读；
- `running`：通过 `event_ref`/TaskMgr 观察后续状态，不能重复创建任务；
- `failed`：读取结构化 `error`，不能只按空结果判断。

取消：

```json
{
  "method": "cancel",
  "params": { "task_id": "aicc-task-id" },
  "sys": [1004, "<session_token>", "trace-cancel-42"]
}
```

响应 `CancelResponse { task_id, accepted }`。`accepted=true` 表示取消已确认；无权限、终态任务或 Provider 未确认取消不能伪装成成功。

## 7. 路由控制

`ModelRequirement` 用于硬需求，例如 `tool_call`、`json_schema`、`web_search`、
`vision`、`image_generation`、`min_context_tokens` 和 canonical field 要求。`ModelDisable` 用于显式禁用能力。

`RoutePolicy` 支持：`cheap | fast | balanced | quality` profile、local only、fallback、runtime failover、explain、Provider allow/block、最大成本和最大延迟。策略只能缩小候选或改变排序，不能赋予模型/adapter 不存在的能力。

session overlay 是请求级临时配置，不能绕过上层 locked policy。显式 exact model 默认不隐式换模型；只有策略明确允许 exact fallback 且服务构造了候选时例外。

## 8. 管理接口

| 目标 | method |
| --- | --- |
| 获取/更新路由配置 | `routing.get`, `routing.update` |
| 原子重载 settings | `service.reload_settings` |
| 列出模型 | `models.list` |
| Provider/adapter catalog | `provider.catalog`, `protocol_adapter.list` |
| 校验/新增 Provider | `provider.validate`, `provider.add` |
| 列表/健康/更新/删除 | `provider.list`, `provider.health`, `provider.update`, `provider.delete` |
| 立即刷新库存 | `provider.refresh_models` |
| 配额/用量/route trace | `quota.query`, `usage.query`, `trace.query` |
| metadata 更新状态/目标 | `driver_metadata_update.get`, `driver_metadata_update.set` |

配置写入使用 `expected_settings_revision` 做 CAS。收到 `settings_revision_conflict` 后必须重新读取、重放用户意图，不能盲目覆盖。`provider.validate` 不写配置；`provider.add/update/delete` 才改变 system-config 并发布新 runtime。

管理权限与普通推理权限分离。所有管理响应必须脱敏。

## 9. Rust 调用示例

```rust
use buckyos_api::{AiMessage, AiRole, AiccClient, LlmChatHelperRequest};

let messages = vec![AiMessage::text(AiRole::User, "你好".to_owned())];
let mut request = LlmChatHelperRequest::new("llm.chat", messages);
request.idempotency_key = Some("chat-42".to_owned());
let response = client.helper_llm_chat(request).await?;
```

也可以先使用 `RouteResolveRequest::new(ApiType::Llm, "llm.chat")`，再用响应中的
`selected_exact_model` 构造 `LlmChatInvokeRequest`。`AiccClient::invoke(AiccCall)` 只覆盖核心路由与推理调用；管理面使用对应 typed client method。

## 10. 错误处理与重试

业务错误可通过 `AiccError::from_krpc_error` 解码。稳定 code 见
[Trait 与 Protocol 冻结设计](frozen_trait_protocol.md)。

重试规则：

- `invalid_*`、schema、permission、policy、budget、idempotency conflict 不自动重试；
- 仅当 `retriable=true` 且请求具有可靠 `idempotency_key` 时重试计费操作；
- 尊重 Provider 的 retry-after；
- Provider 接受任务后的超时不等于任务不存在，应通过 TaskMgr/event 观察；
- 不按 `message`、HTTP 状态或 Provider 私有错误正文做业务分支。

## 11. 冻结后的变更规则

method 名、request/response 字段、enum wire 值、逻辑/exact model 格式、资源 tag、任务状态和错误 code 均是用户合同。任何修改必须同步 Rust SDK、TypeScript 声明、dispatcher、本文、`aicc_api设计.md` 和 DV/contract tests。Beta 2.2 直接切换，不保留旧字段 alias。
