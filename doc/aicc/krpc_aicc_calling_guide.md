# AICC kRPC 调用指南（含 Mock Provider 对接）

## 1. 目标与范围

本文说明如何通过 kRPC 调用 AICC 下发 AI 任务，覆盖：

- AICC 服务入口与 method
- kRPC 报文结构（`sys`）
- `complete` / `cancel` 请求示例
- 返回语义
- OpenAI / Gemini / MiniMax 三类 provider 对接 mock 接口的方法

## 2. 服务入口与可用方法

AICC HTTP 入口：

- `POST /kapi/aicc`

AICC 当前对外业务方法：

- `complete`
- `cancel`

## 3. kRPC 报文结构（重点）

AICC 使用 kRPC 协议，不是 JSON-RPC 的 `id` 字段。

请求结构：

- `method`: 方法名
- `params`: 参数对象
- `sys`: 数组，约定为 `[seq, token?, trace_id?]`

示例：

```json
{
  "method": "complete",
  "params": {"...": "..."},
  "sys": [1001, "<session_token>", "trace-abc"]
}
```

说明：

- `sys[0]`：`seq`（u64）
- `sys[1]`：`token`（可选；若要传 `trace_id` 但无 token，需要填 `null`）
- `sys[2]`：`trace_id`（可选）

## 4. 下发 AI 任务：`complete`

### 4.1 最小可用请求（LLM）

```json
{
  "method": "complete",
  "params": {
    "capability": "llm_router",
    "model": {
      "alias": "llm.plan.default"
    },
    "requirements": {
      "must_features": ["plan"],
      "max_latency_ms": 3000,
      "max_cost_usd": 0.2
    },
    "payload": {
      "messages": [
        {
          "role": "user",
          "content": "请给我写一段发布说明"
        }
      ],
      "options": {
        "temperature": 0.2,
        "max_tokens": 256,
        "session_id": "s-001",
        "rootid": "s-001"
      }
    },
    "idempotency_key": "idem-20260322-001"
  },
  "sys": [1001, "<session_token>", "trace-aicc-001"]
}
```

### 4.2 字段说明

- `capability`
  - 常见：`llm_router`
  - 也支持如 `text2_image` 等能力（取决于 provider/路由映射）
- `model.alias`
  - 推荐用平台映射别名，如 `llm.default`、`llm.plan.default`
- `requirements.must_features`
  - 如：`plan`、`json_output`、`tool_calling`、`web_search`
- `payload.messages`
  - 标准多轮消息；若不传，可用 `payload.text`
- `payload.options`
  - 透传给 provider 适配层（会按各 provider 支持项筛选）
- `idempotency_key`
  - 推荐传，便于幂等追踪

补充：AICC 会把 `session_id/rootid` 写入任务数据，未传时会按策略生成默认 `rootid`。

## 5. 取消任务：`cancel`

```json
{
  "method": "cancel",
  "params": {
    "task_id": "aicc_xxx"
  },
  "sys": [1002, "<session_token>", "trace-aicc-002"]
}
```

说明：

- 若任务不存在绑定或 provider 不支持取消，可能返回 `accepted=false`
- AICC 有租户校验：跨租户取消会拒绝（`NoPermission`）

## 6. 返回语义

`complete` 成功响应体（`result`）结构：

- `task_id: string`
- `status: "succeeded" | "running" | "failed"`
- `result?: AiResponseSummary`（仅同步成功时通常会带）
- `event_ref?: string`

示例：

```json
{
  "result": {
    "task_id": "aicc_123",
    "status": "running",
    "event_ref": "task://aicc_123/events"
  },
  "sys": [1001, "trace-aicc-001"]
}
```

`cancel` 成功响应体（`result`）结构：

- `task_id: string`
- `accepted: boolean`

## 7. Mock Provider 对接（你已有模拟接口时）

核心思路：在 AICC settings 中把 provider `base_url` 指向你的 mock 服务。

### 7.1 OpenAI 适配器

AICC(OpenAI) 会调用：

- LLM：`POST {base_url}/responses`
- 文生图：`POST {base_url}/images/generations`

### 7.2 Gemini 适配器

AICC(Gemini/Gimini) 会调用：

- `POST {base_url}/models/{model}:generateContent`
- Header: `x-goog-api-key: <api_token>`

### 7.3 MiniMax 适配器

AICC(MiniMax) 会调用：

- `POST {base_url}/messages`
- Header: `x-api-key: <api_token>`

## 8. Settings 样例（指向本地 mock）

```json
{
  "openai": {
    "enabled": true,
    "api_token": "mock-openai-token",
    "instances": [
      {
        "provider_instance_name": "openai-mock-1",
        "provider_type": "cloud_api",
        "provider_driver": "openai",
        "base_url": "http://127.0.0.1:18080/v1",
        "models": ["gpt-4o-mini"],
        "default_model": "gpt-4o-mini"
      }
    ]
  },
  "gimini": {
    "enabled": true,
    "api_key": "mock-gemini-key",
    "instances": [
      {
        "provider_instance_name": "gemini-mock-1",
        "provider_type": "cloud_api",
        "provider_driver": "google-gimini",
        "base_url": "http://127.0.0.1:18081/v1beta",
        "models": ["gemini-2.5-flash"],
        "default_model": "gemini-2.5-flash"
      }
    ]
  },
  "minimax": {
    "enabled": true,
    "api_token": "mock-minimax-token",
    "instances": [
      {
        "provider_instance_name": "minimax-mock-1",
        "provider_type": "cloud_api",
        "provider_driver": "minimax",
        "base_url": "http://127.0.0.1:18082/anthropic/v1",
        "models": ["MiniMax-M2.5"],
        "default_model": "MiniMax-M2.5"
      }
    ]
  }
}
```

说明：

- `openai.enabled` 默认可开启；`minimax.enabled` 默认是 `false`，要显式置 `true`
- `gimini` 同时兼容 `gemini/google_gimini/google` 键名
- 配置更新后由服务端配置机制生效，无需额外调用 AICC 热加载方法

## 9. 快速联调建议

1. 先用 `complete` 打通 `llm_router + llm.plan.default`。
2. 如果返回 `failed`，先看错误码是否为：
   - `no_provider_available`（路由不到可用模型）
   - `resource_invalid`（资源/入参不合法）
   - `provider_start_failed`（provider 启动或上游调用失败）
3. mock 接口先保证最小响应可解析，再逐步补全 usage/tool_calls/artifacts。

## 10. 参考代码位置

- `src/frame/aicc/src/main.rs`
- `src/frame/aicc/src/aicc.rs`
- `src/frame/aicc/test_llm.py`
- `src/kernel/buckyos-api/src/aicc_client.rs`
- `d:/rust/.cargo/git/checkouts/buckyos-base-2e78dd85e20cc97b/ea4f35a/src/kRPC/src/protocol.rs`
