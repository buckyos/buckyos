# 通过 system_config 更新 AICC 配置

本文说明如何把 AICC Provider settings 写入 `system_config`，并触发 AICC 在线重载。新版模型路由以 `doc/aicc/aicc_router.md` 为准：Provider settings 只负责声明 Provider instance、部署类型、凭据、endpoint 和模型清单；逻辑模型选择由 Provider inventory、默认逻辑目录配置、`system_config` 中的 AICC 系统配置和 request/session 级 overlay 完成。

## 1. 配置位置

AICC settings 存储在：

```text
services/aicc/settings
```

AICC 启动和 `service.reload_settings` 时读取该 key，并原子重建 Provider registry 与 ModelRegistry。

## 2. Settings 基本结构

```json
{
  "providers": [
    {
      "provider_instance_name": "openai-primary",
      "provider_type": "cloud_api",
      "provider_profile_id": "openai",
      "protocol_adapter_id": "openai-responses",
      "endpoint": "https://api.openai.com/v1",
      "credentials": {
        "api_token": { "locked": "..." }
      },
      "region": null,
      "pricing_context": null,
      "provider_rules_id": "openai"
    }
  ],
  "session_config": {}
}
```

`provider_instance_name` 是稳定主键；Profile、Protocol Adapter 和 Model Driver 是不同身份。Provider Instance 不保存静态模型能力或逻辑挂载，catalog 更新也不能修改 endpoint、凭据、区域和协议选择。

Beta 2.2 不读取 Provider family section、`instances[]` 包装、`provider_driver`、`base_url`、`features`、`alias_map`、section 级 token 或字段别名。
## 3. 更新方式

通过 `system_config` kRPC 更新：

- 全量覆盖：`sys_config_set`
- 局部更新：`sys_config_set_by_json_path`
- 事务更新：`sys_config_exec_tx`，按需要使用

下列示例均为：

```text
POST /kapi/system_config
```

请求体使用 kRPC 结构：`method`、`params`、`sys`。

## 4. 读取当前配置

```json
{
  "method": "sys_config_get",
  "params": {
    "key": "services/aicc/settings"
  },
  "sys": [3001, "<session_token>", "trace-aicc-cfg-get"]
}
```

建议变更前先备份旧值。

## 5. 全量覆盖

`value` 是字符串，不是对象；内部 JSON 需要先序列化。

```json
{
  "method": "sys_config_set",
  "params": {
    "key": "services/aicc/settings",
    "value": "{\"providers\":[{\"provider_instance_name\":\"openai-primary\",\"provider_type\":\"cloud_api\",\"provider_profile_id\":\"openai\",\"protocol_adapter_id\":\"openai-responses\",\"endpoint\":\"https://api.openai.com/v1\",\"credentials\":{\"api_token\":{\"locked\":\"...\"}},\"provider_rules_id\":\"openai\"}]}"
  },
  "sys": [3002, "<session_token>", "trace-aicc-cfg-set"]
}
```

## 6. 局部更新

示例：只更新 `/openai` 节点。

```json
{
  "method": "sys_config_set_by_json_path",
  "params": {
    "key": "services/aicc/settings",
    "json_path": "/providers/0",
    "value": "{\"provider_instance_name\":\"openai-primary\",\"provider_type\":\"cloud_api\",\"provider_profile_id\":\"openai\",\"protocol_adapter_id\":\"openai-responses\",\"endpoint\":\"https://api.openai.com/v1\",\"credentials\":{\"api_token\":{\"locked\":\"...\"}},\"provider_rules_id\":\"openai\"}"
  },
  "sys": [3003, "<session_token>", "trace-aicc-cfg-patch"]
}
```

## 7. 触发 AICC 重载

写入 system_config 后调用：

```text
POST /kapi/aicc
```

```json
{
  "method": "service.reload_settings",
  "params": {},
  "sys": [3004, "<session_token>", "trace-aicc-reload"]
}
```

兼容 method：

- `service.reload_settings`

成功响应：

```json
{
  "result": {
    "ok": true,
    "providers_registered": 1
  },
  "sys": [3004, "trace-aicc-reload"]
}
```

## 8. 验证配置已生效

先查模型目录：

```json
{
  "method": "models.list",
  "params": {},
  "sys": [3005, "<session_token>", "trace-aicc-models"]
}
```

确认返回中包含：

- Provider instance：`openai-primary`
- 模型 exact model：例如 `gpt-5.2@openai-primary`
- 目标 `logical_mounts`：例如 `llm.chat`、`llm.openai`、`llm.gpt5`

再发最小 AI 调用：

```json
{
  "method": "llm.chat",
  "params": {
    "capability": "llm",
    "model": {
      "alias": "llm.chat"
    },
    "requirements": {},
    "payload": {
      "messages": [
        {
          "role": "user",
          "content": "ping"
        }
      ]
    }
  },
  "sys": [3006, "<session_token>", "trace-aicc-ping"]
}
```

强制指定 Provider 验证：

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
          "content": "ping"
        }
      ]
    }
  },
  "sys": [3007, "<session_token>", "trace-aicc-exact"]
}
```

## 9. 常见错误

- `no_provider_available`：Provider 未注册、`logical_mounts` 不包含目标逻辑目录、Provider 不可用、策略过滤后无候选。
- `logical_model_not_found`：目标逻辑模型不存在或没有可用候选。
- `max_cost_exceeded`：所有候选超过 `requirements.max_cost_usd`。
- `resource_invalid`：payload resources 格式不合法。
- `provider_start_failed`：Provider 已选中，但上游调用失败。

排查顺序：

1. `sys_config_get` 确认 settings 已写入。
2. `service.reload_settings` 确认 `providers_registered > 0`。
3. `models.list` 查看 inventory、exact model 和 `logical_mounts`。
4. 先用 typed inference 的 `exact_model` 验证 Provider，再用 `route.resolve.logical_model` 验证路由。

## 10. 注意事项

- `sys_config_set` 和 `sys_config_set_by_json_path` 受 RBAC 控制，token 需要有 `services/aicc/settings` 写权限。
- `value` 必须是字符串；调用端负责 JSON 序列化。
- `provider_type=local_inference` 具有安全含义，只能用于可信本地推理实例。
- 不确定部署边界的代理服务使用 `proxy_unknown`，不要伪装成本地推理。
- AICC 当前不支持 per-user routing config。系统级 routing 配置持久化在 `services/aicc/settings.routing_config`，例如 `provider_weights`、`global_exact_model_weights`、`policy`、`logical_tree` 和 `logical_definitions`；Ai Center UI 的系统级调整写入这里。调用方如果需要临时调整 provider/model 偏好，应在每次 RPC 的 request 级 `session_overlay` 中表达。
- `models.list` 返回“默认逻辑目录配置 + system_config 中 AICC 系统配置”的合并视图，不包含 per-session overlay。
- 变更后调用 `service.reload_settings`；只写 system_config 不会让运行中的 AICC 立即重建 Provider registry。

## 11. 参考代码

- `src/frame/aicc/src/main.rs`
- `src/frame/aicc/src/aicc.rs`
- `src/frame/aicc/src/model_registry.rs`
- `src/frame/aicc/src/model_session.rs`
- `src/frame/aicc/src/model_types.rs`
- `src/frame/aicc/src/openai.rs`
- `src/frame/aicc/src/claude.rs`
- `src/frame/aicc/src/gemini.rs`
- `src/frame/aicc/src/minimax.rs`
- `src/kernel/buckyos-api/src/aicc_client.rs`
- `src/kernel/buckyos-api/src/system_config.rs`
