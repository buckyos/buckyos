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
    "api_type": "llm.chat",
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
      "protocol_adapter_id": "openai",
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

typed inference 只接受 `exact_model`，不做隐式模型 fallback。Model Driver 和 Provider Rules 生成的厂商参数只存在于内部 `ResolvedProviderCall`，调用方不得传入 `provider_options`。

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

## 3. Typed inference methods

- LLM：`chat.completions.create`
- Embedding：`embeddings.create`
- Rerank：`rerank.create`
- Image：`images.generate`、`images.edit`、`images.upscale`、`images.remove_background`
- Vision：`vision.ocr`、`vision.caption`
- Audio：`audio.speech.create`、`audio.transcriptions.create`、`audio.music.create`、`audio.enhance`
- Video：`videos.generate`、`videos.transform`、`videos.extend`、`videos.upscale`
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

`route.resolve` 可以返回有序候选；某个 exact model 调用失败后，由调用方重新选择候选或重新路由。typed inference 本身不静默换模型。
