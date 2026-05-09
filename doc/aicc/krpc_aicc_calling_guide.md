# AICC kRPC 调用指南

本文说明如何通过 kRPC 调用新版 AICC。新版模型路由以 `doc/aicc/aicc_router.md` 为准：调用方传入逻辑模型名或精确模型名，AICC 根据 Provider inventory、默认逻辑目录树、session config 和 route policy 解析候选并调度。

## 1. 服务入口

AICC HTTP kRPC 入口：

- `POST /kapi/aicc`

AICC 使用 kRPC 协议，请求体不是 JSON-RPC。基本结构为：

```json
{
  "method": "llm.chat",
  "params": {},
  "sys": [1001, "<session_token>", "trace-aicc-001"]
}
```

`sys` 约定：

- `sys[0]`：`seq`
- `sys[1]`：`session_token`，可选
- `sys[2]`：`trace_id`，可选；如果无 token 但要传 trace_id，`sys[1]` 填 `null`

## 2. Method

AI 调用 method 直接使用能力方法名，不再使用旧的 `complete` RPC method。

常用 method：

- `llm.chat`
- `llm.completion`
- `embedding.text`
- `rerank`
- `image.txt2img`
- `image.img2img`
- `image.upscale`
- `image.bg_remove`
- `vision.ocr`
- `vision.caption`
- `audio.tts`
- `audio.asr`
- `video.txt2video`
- `agent.computer_use`

管理 method：

- `cancel`
- `reload_settings` / `service.reload_settings`
- `models.list` / `service.models.list`

## 3. 最小 LLM 请求

```json
{
  "method": "llm.chat",
  "params": {
    "capability": "llm",
    "model": {
      "alias": "llm.plan"
    },
    "requirements": {
      "must_features": ["plan"],
      "max_latency_ms": 3000,
      "max_cost_usd": 0.2,
      "extra": {
        "allow_fallback": true,
        "runtime_failover": true
      }
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
        "session_id": "s-001"
      }
    },
    "idempotency_key": "idem-20260322-001"
  },
  "sys": [1001, "<session_token>", "trace-aicc-001"]
}
```

说明：

- `model.alias` 可以是逻辑模型名，例如 `llm.plan`、`llm.chat`、`llm.gpt5`；也可以是精确模型名，例如 `gpt-5.2@openai-primary`。
- 精确模型名格式是 `<provider_model_id>@<provider_instance_name>`，默认表达“强制指定 Provider”，默认不做精确模型 fallback。
- `requirements.must_features` 是硬过滤条件，常见值包括 `plan`、`tool_calling`、`json_output`、`web_search`、`vision`。
- `requirements.max_cost_usd` 会参与动态成本过滤。
- `payload.options.session_id` 会启用同一 session 内的路由粘性。
- 当前实现中 request 级路由控制字段从 `requirements.extra`、`payload.options` 或 `payload.input_json` 读取；顶层 `policy` 字段暂不作为路由决策来源。
- 更细的调度 profile 通过 `SessionConfig.policy.profile` 配置，可用值包括 `cost_first`、`latency_first`、`quality_first`、`balanced`、`local_first`、`strict_local`。

## 4. 精确模型请求

用于调试、Provider 对比或强制指定供应商：

```json
{
  "method": "llm.chat",
  "params": {
    "capability": "llm",
    "model": {
      "alias": "gpt-5.2@openai-primary"
    },
    "requirements": {
      "extra": {
        "allow_fallback": false,
        "runtime_failover": false
      }
    },
    "payload": {
      "messages": [
        {
          "role": "user",
          "content": "用一句话解释 AICC 路由"
        }
      ]
    }
  },
  "sys": [1002, "<session_token>", "trace-aicc-exact"]
}
```

如果精确模型不可用且未显式允许精确模型 fallback，AICC 会返回路由错误。

## 5. Request 级 SessionConfig Patch

调用方可以在 `requirements.extra`、`payload.options` 或 `payload.input_json` 中携带控制字段。常用字段：

- `session_config`：替换当前 session 的完整 `SessionConfig`
- `session_config_patch`：基于当前 session config 做局部合并
- `expected_session_config_revision` / `expected_revision`：并发更新校验
- `local_only`
- `allow_fallback`
- `runtime_failover`

示例：只允许本地候选，并把 `llm.plan` 的 fallback 改成严格模式：

```json
{
  "method": "llm.chat",
  "params": {
    "capability": "llm",
    "model": {
      "alias": "llm.plan"
    },
    "requirements": {
      "extra": {
        "local_only": true,
        "session_config_patch": {
          "policy": {
            "local_only": true
          },
          "logical_tree": {
            "llm": {
              "children": {
                "plan": {
                  "fallback": {
                    "mode": "strict"
                  }
                }
              }
            }
          }
        }
      }
    },
    "payload": {
      "messages": [
        {
          "role": "user",
          "content": "总结这段本地资料"
        }
      ],
      "options": {
        "session_id": "local-session-001"
      }
    }
  },
  "sys": [1003, "<session_token>", "trace-aicc-local"]
}
```

## 6. 取消任务

```json
{
  "method": "cancel",
  "params": {
    "task_id": "aicc-xxx"
  },
  "sys": [1004, "<session_token>", "trace-aicc-cancel"]
}
```

说明：

- 若任务不存在绑定或 Provider 不支持取消，可能返回 `accepted=false`。
- 跨租户取消会被拒绝。

## 7. 返回结构

AI method 成功响应的 `result` 结构：

- `task_id`
- `status`：`succeeded`、`running` 或 `failed`
- `result`：同步完成时返回 `AiResponseSummary`
- `event_ref`：异步任务事件引用

示例：

```json
{
  "result": {
    "task_id": "aicc-1710000000000-1",
    "status": "succeeded",
    "result": {
      "text": "AICC 会把逻辑模型名解析为候选模型，再按策略选择 Provider。",
      "finish_reason": "stop",
      "extra": {
        "route_summary": {
          "display_name": "gpt-5.2 (openai-primary)"
        }
      }
    }
  },
  "sys": [1001, "trace-aicc-001"]
}
```

常见错误码：

- `no_provider_available`：无候选、Provider 不可用、策略过滤后为空
- `model_alias_not_mapped`：逻辑模型名没有命中目录或 legacy catalog 兼容映射
- `max_cost_exceeded`：所有候选都超过预算
- `context_too_long`
- `resource_invalid`
- `provider_start_failed`

## 8. 查询模型目录

调试路由时优先调用：

```json
{
  "method": "models.list",
  "params": {},
  "sys": [1005, "<session_token>", "trace-aicc-models"]
}
```

返回中会包含当前 Provider inventory、模型 logical mounts、默认 session config 和 legacy aliases。新增路由应优先依赖 inventory 的 `logical_mounts` 和 `SessionConfig`，legacy aliases 只作为兼容层。

## 9. Mock Provider 对接

Mock 的核心是把 AICC settings 中对应 Provider instance 的 `base_url` 指向本地 mock 服务，然后调用 `reload_settings`。

OpenAI 适配器会调用：

- LLM：`POST {base_url}/responses`
- 文生图：`POST {base_url}/images/generations`

Gemini 适配器会调用：

- `POST {base_url}/models/{model}:generateContent`
- Header：`x-goog-api-key: <api_token>`

MiniMax 适配器会调用：

- `POST {base_url}/messages`
- Header：`x-api-key: <api_token>`

Settings 片段：

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
        "models": ["gpt-5-mini"],
        "default_model": "gpt-5-mini"
      }
    ]
  }
}
```

最小联调建议：

1. 先调用 `models.list`，确认 mock provider 的模型出现在 inventory，且有 `llm.chat` 或目标 logical mount。
2. 再用 `llm.chat + model.alias=llm.chat` 打通路由。
3. 如果要验证强制指定 Provider，用 `model.alias=<model>@<provider_instance_name>`。
