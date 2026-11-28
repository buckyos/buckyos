# AICC kRPC 调用指南

> 状态：Beta 2.2 目标规范。本文描述重构完成后的调用方式，不保留旧 all-in-one method、字段别名或兼容入口。

完整类型定义以 [`../aicc_api设计.md`](../aicc_api设计.md) 为准，路由语义以 [`../aicc_router.md`](../aicc_router.md) 为准。

## 1. 服务入口与信封

AICC 的 kRPC 入口是 `POST /kapi/aicc`。请求不是 JSON-RPC，统一信封如下：

```json
{
  "method": "route.resolve",
  "params": {},
  "sys": [1001, "<session_token>", "trace-aicc-001"]
}
```

- `sys[0]`：请求序号。
- `sys[1]`：可选 `session_token`；无 token 时填 `null`。
- `sys[2]`：可选 `trace_id`。

RPC `method` 是公开方法名；`api_type` 是路由操作类型。两者不得混用。

## 2. 两种调用方式

### 2.1 route + typed inference

先解析逻辑模型：

```json
{
  "method": "route.resolve",
  "params": {
    "api_type": "llm",
    "logical_model": "llm.plan",
    "requirements": { "tool_call": true, "json_schema": true },
    "policy": { "profile": "quality" }
  },
  "sys": [1001, "<session_token>", "trace-aicc-route"]
}
```

路由结果必须给出：

```json
{
  "selected_exact_model": "gpt-5.1:reasoning-high@openai-primary",
  "selected_model_uid": "openai/gpt-5.1/reasoning-high",
  "provider_instance_name": "openai-primary",
  "provider_profile_id": "openai",
      "protocol_adapter_id": "openai-responses",
  "model_driver_id": "openai-gpt-5",
  "origin_model_id": "gpt-5.1",
  "provider_model_id": "gpt-5.1",
  "operation": "responses.create"
}
```

再调用 typed inference：

```json
{
  "method": "chat.completions.create",
  "params": {
    "exact_model": "gpt-5.1:reasoning-high@openai-primary",
    "messages": [{
      "role": "user",
      "content": [{ "type": "text", "text": "写一段发布说明" }]
    }]
  },
  "sys": [1002, "<session_token>", "trace-aicc-chat"]
}
```

typed inference 只接受 `exact_model`，从不切换模型或 Provider：即使 `exact_model` 是 `route.resolve` 按用途表选出来的，用途的“不可用时”策略也不再生效。要让策略生效，用 Helper。Model Driver 和 Provider Rules 生成的厂商参数只存在于内部 `ResolvedProviderCall`，调用方不得传入 `provider_options`。

### 2.2 Helper

Helper 接受逻辑模型和对应业务字段，内部完成一次 route + typed inference：

```json
{
  "method": "helper.llm_chat",
  "params": {
    "logical_model": "llm.plan",
    "requirements": { "tool_call": true, "json_schema": true },
    "messages": [{
      "role": "user",
      "content": [{ "type": "text", "text": "写一段发布说明" }]
    }]
  },
  "sys": [1003, "<session_token>", "trace-aicc-helper"]
}
```

`logical_model` 是用途（`llm.plan` 等）时，模型由对调用者生效的用途表决定：默认 < 管理员层 < 用户层 < App 绑定的策略组 < 会话绑定的策略组（[aicc_router.md §11](../aicc_router.md#11-用途表与配置层次)）。规格、家族路径（如 `llm.gpt-mini`）不读用途表，按静态展开。

### 2.3 策略组与请求策略

`route.resolve`、`helper.llm_chat`、`helper.text_to_image`、`helper.video_generate` 可以带：

- `route_group`：路由策略组 ID（`rg_…`），把这个组作为本次请求的会话层。组必须属于本次调用的额度用户；不属于时当作没传，trace 的 `usecase.route_group_ignored` 为 `true`，不报错。Agent 会话由 lib_opendan 沿 `parent_session` 链取最近一个设了 `route_group` 的会话并自动带上。
- `policy`：只能收紧，不能放宽管理员的配置（[aicc_router.md §12.3](../aicc_router.md#12-控制面设计)）。`policy.no_fallback: true` 关闭本次请求的“不可用时”策略；其余可用项是 `local_only`、`blocked_provider_instances`、`allowed_provider_instances`（与管理员的允许列表同时生效）、更低的 `max_cost`、`max_latency_ms`、`explain`，以及管理员没有锁定时的 `profile`（`fast` / `balanced` / `quality`）。

```json
{
  "method": "helper.llm_chat",
  "params": {
    "logical_model": "llm.code",
    "route_group": "rg_xxx",
    "policy": { "no_fallback": true },
    "messages": [{ "role": "user", "content": [{ "type": "text", "text": "ping" }] }]
  },
  "sys": [1005, "<session_token>", "trace-aicc-group"]
}
```

请求里的 `session_overlay`、`policy.allow_fallback`、`policy.runtime_failover` 和 `profile: "cheap"` 已删除，带这些字段的请求会因未知字段被拒绝。

## 3. Typed inference methods

- LLM：`chat.completions.create`
- Embedding：`embedding.text`、`embedding.multimodal`
- Rerank：`rerank`
- Image：`images.generate`、`image.img2img`、`image.inpaint`、`image.upscale`、`image.bg_remove`
- Vision：`vision.ocr`、`vision.caption`
- Audio：`audio.tts`、`audio.asr`、`audio.music`、`audio.enhance`
- Video：`video.generate`、`video.edit`、`video.extend`、`video.upscale`（文本、首尾帧和参考素材都是 `video.generate` 的 `conditions[]`；逻辑模型用 `helper.video_generate`；结果为 `videos[]`。见 [video_generation.md](../video_generation.md)）
- Agent：`agent.computer_use`

资源输入统一使用 `ResourceRef`，输出统一使用 artifact/FileObject 引用。业务结果不得塞入脱敏后的诊断字段。

## 4. 任务与取消

```json
{
  "method": "cancel",
  "params": { "task_id": "aicc-xxx" },
  "sys": [1004, "<session_token>", "trace-aicc-cancel"]
}
```

只有 Provider 已确认取消时才可报告取消成功。若上游不能取消，AICC 应本地停止轮询并返回未确认状态；迟到的 Provider Final 不得覆盖已终止的本地任务状态。跨租户取消必须拒绝。

## 5. 管理与诊断

- `service.reload_settings`：重新加载 Provider Instance 和路由配置。
- `models.list`：查询逻辑模型、精确模型、渠道身份、operations、能力和健康摘要。
- `provider.catalog`：查询 Provider Profile 与 Provider Rules。
- `protocol_adapter.list`：查询已注册的 Protocol Adapter。
- `provider.validate`：校验 Provider Instance 草案，不写入 system-config。

所有管理结果必须脱敏，不能返回凭据或内部签名材料。模型能力由 Model Driver、Protocol Adapter 和 discovery 结果取交集；未知能力不得通过模型名猜测。

## 6. 错误与重试

常见稳定错误包括 `no_provider_available`、`logical_model_not_found`、`exact_model_not_found`、`operation_not_supported`、`context_too_long`、`resource_invalid`、`provider_start_failed` 和 `cancel_not_confirmed`。

调用失败时 AICC 先对同一精确模型重试，然后返回 Provider 的真实错误（Provider 实例名、精确模型和上游原因）；不自动换模型或 Provider，也不把故障改写成 `no_candidate_model`。`no_candidate_model` 只表示没有接入满足请求的模型，错误详情带 `usecase`。

只有通过 Helper 按用途路由、且该用途的“不可用时”策略不是 `none`、本次请求没有设 `no_fallback` 时，触发类故障（重试后仍超时 / 5xx / 连接失败、模型已下线、上游配额耗尽、用户额度规则拒绝该模型、Provider 被管理员标记为故障）才会换一次模型；备选也失败时，错误的 `details.fallback.from` / `details.fallback.to` 同时给出两边的原因。400、401、403、409、取消、已产生用量的失败、异步任务已拿到 remote task ID 之后的失败从不切换。typed inference 的 exact model 从不切换（[aicc_router.md §9](../aicc_router.md#9-fallback-与失败处理)）。

Provider 被标记为故障且用途没有配置策略时，请求立即返回 `no_provider_available`，`details.reason` 为 `marked_faulty`。

响应的 `route_trace.fallback` 标出答案不是来自用途自己的模型：`kind: "unavailable"`（策略切换，`from` / `to` 是两个精确模型）或 `kind: "coverage"`（用途没有模型，由兜底 LLM `llm.fallback` 提供）。调用方据此提示用户，不要从 `extra` 里猜。
