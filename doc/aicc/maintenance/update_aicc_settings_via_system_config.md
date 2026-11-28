# 通过 system_config 更新 AICC 配置

本文说明如何把 AICC Provider settings 写入 `system_config`，并触发 AICC 在线重载。新版模型路由以 `doc/aicc/aicc_router.md` 为准：Provider settings 只负责声明 Provider instance、部署类型、凭据、`base_url` 和 discovery 配置；模型清单来自 Provider inventory；用途选哪个模型由用途表决定（默认 < 管理员层 < 用户层 < App 绑定的策略组 < 会话绑定的策略组，见 [aicc_router.md §11](../aicc_router.md#11-用途表与配置层次)）。settings 里只有管理员层，其余各层存在 AICC 自己的 RDB 里，不在 system-config。

普通 AI Center 前端不得直接使用本文的 system-config 接口，而应调用 AICC `provider.add/update/delete` 管理 API，由 AICC 使用当前调用者 token 和 revision CAS 受控写入。本文的直接写法只供具备相应 RBAC 权限的管理员维护、调试和外部工具使用；无论通过哪条入口，system-config 都是唯一配置真相源。

## 1. 配置位置

AICC settings 存储在：

```text
services/aicc/settings
```

AICC 启动和 `service.reload_settings` 时读取该 key，并原子重建 Provider registry 与 ModelRegistry。

## 2. Settings 基本结构

```json
{
  "schema": "buckyos.aicc.settings.v1",
  "providers": [
    {
      "provider_instance_name": "openai-primary",
      "provider_type": "cloud_api",
      "provider_profile_id": "openai",
      "protocol_adapter_id": "openai-responses",
      "base_url": "https://api.openai.com/v1",
      "credentials": {
        "api_token": { "inline_secret": "..." }
      },
      "region": null,
      "provider_rules_id": "openai"
    }
  ],
  "routing": {}
}
```

`provider_instance_name` 是稳定主键；Profile、Protocol Adapter 和 Model Driver 是不同身份。Provider Instance 只保存 `provider_profile_id` 和 `protocol_adapter_id`，模型级 `model_driver_id` 由 catalog/inventory 解析产生。Provider Instance 不保存静态模型能力或逻辑挂载，catalog 更新也不能修改 `base_url`、凭据、区域和协议选择。

Provider 的 `timeout_ms` 缺省或为 `null` 时采用 `600000` 毫秒（10 分钟），显式设置时使用该值。

SN 使用独立 `sn-openai` Adapter，并显式选择认证模式。静态 API Key 示例：

```json
{
  "provider_instance_name": "sn-main",
  "provider_profile_id": "sn",
  "protocol_adapter_id": "sn-openai",
  "base_url": "https://sn.example/v1",
  "auth": {
    "mode": "api_key",
    "credential_ref": "system-config://secrets/aicc/sn-main"
  }
}
```

运行时登录示例：

```json
{
  "provider_instance_name": "sn-main",
  "provider_profile_id": "sn",
  "protocol_adapter_id": "sn-openai",
  "base_url": "https://sn.example/v1",
  "auth": {
    "mode": "dynamic_login",
    "login_profile": "device_jwt",
    "login_endpoint": "https://sn.example/api/user/login_by_device_token"
  }
}
```

两种模式互斥。动态 token 只由 `sn-openai` 在运行时缓存和刷新，不写回 system-config，也不由 `openai` 基础 Adapter 处理。

GLM 默认使用 Bearer。需要由 AICC 从 `key-id.secret` 生成短期 JWT 时，在静态认证中显式增加 `"credential_kind": "glm_jwt"`；其它 Provider 请求未声明的 credential kind 会被拒绝。

Beta 2.2 不读取 Provider family section、`instances[]` 包装、`provider_driver`、`endpoint`、`features`、`alias_map`、section 级 token 或字段别名。`base_url` 是新旧 settings 都保留的正式字段。

Provider 条目还可以带 `"marked_faulty": true`：管理员手工把它标记为故障。被标记的 Provider 仍参与路由、仍显示在用途表里，但请求不再发给它；配置了“不可用时”策略的用途直接改用备选，没配置的立即返回 `no_provider_available`（见 [aicc_router.md §9.6](../aicc_router.md#9-fallback-与失败处理)）。字段为 `false` 时不写。优先用 `provider.update {"marked_faulty": true|false}` 修改：只改这一项时不重新验证 Provider，并会记 `provider_marked_faulty` / `provider_fault_cleared` 事件；直接写 system-config 不记事件。长期故障应停用 Provider（`enabled: false`），默认路由随之重算。

### 2.1 `routing`：管理员的路由配置

`routing` 字段（旧名 `session_config`；settings 拒绝未知字段，仍写旧名的文档会加载失败）是 `AiccRouteOverlay`，即用途表的管理员层加上管理员的高级调权手段：

| 字段 | 含义 |
| --- | --- |
| `usecases` | 管理员层（Zone 默认）：`用途 → {model?, on_unavailable?, enabled?}`，见下 |
| `provider_weights` | Provider 实例权重，进入调度评分的 `preference` 分量；`routing.update` 可改 |
| `routing_commands` | 路由调整命令（逻辑目录内 item 的权重）；`routing.update` 可改 |
| `global_exact_model_weights`、`logical_tree` | 精确模型权重、逻辑目录覆盖（`items` / `item_overrides` / `exact_model_weights` / `policy` 等）；只能直接写 settings |
| `policy` | Zone 的路由策略（`profile`、`scheduler_profiles`、`local_only`、`blocked_provider_instances`、`allowed_provider_instances`、`max_estimated_cost`、`explain`），可用 `{"value": ..., "locked": true}` 锁定；请求只能在此之上收紧（[aicc_router.md §12.3](../aicc_router.md#12-控制面设计)） |

已删除的旧字段：`policy` 里的 `allow_fallback`、`allow_exact_model_fallback`、`runtime_failover` 和 `profile: "cost_first"` 会被拒绝；`scheduler_profiles.cost_first`、逻辑目录节点的 `fallback`，以及 `inherit`、`logical_profile(s)`、`active_logical_profile`、`revision`、`ttl_seconds` 不再有任何作用。默认 profile 是 `balanced`。

`usecases` 示例：

```json
{
  "routing": {
    "usecases": {
      "llm.code": {
        "model": { "value": "gpt-codex-5.2@openai-primary", "locked": true },
        "on_unavailable": "same_model"
      },
      "llm.summarize": { "on_unavailable": "not_pricier" },
      "image.txt2img": { "model": "flux-2-pro@fal" },
      "llm.fallback": { "enabled": true }
    }
  }
}
```

- `model`：精确模型 `model@provider`，必须在库存里、API 类型与用途一致、满足用途的硬要求；不写表示跟随默认展开。
- `on_unavailable`：`none`（默认，不切换）/ `same_model` / `not_pricier` / `next_model`，含义见 [aicc_router.md §9.3](../aicc_router.md#9-fallback-与失败处理)。
- 两个字段都可以写成 `"x"` 或 `{"value": "x", "locked": true}`；锁定后用户层和策略组不能改这一格。
- `enabled` 只用于 `llm.fallback`（兜底 LLM），默认关闭；打开后，七个 LLM 用途中静态上没有模型的由它提供。
- 所选模型从库存消失时配置保留，这一格跟随默认，并记 `usecase_model_missing` 事件。

修改用途优先用 AICC 的 `routing.usecase.set`（`target: "zone"`，带 `revision` 做并发检查，写入前按上面的规则校验并记 `usecase_updated` 事件），或 `buckyos ai usecase set ... --zone-default`。直接写 system-config 跳过这些校验，写错的格子会在路由时被跳过。
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
    "value": "{\"schema\":\"buckyos.aicc.settings.v1\",\"providers\":[{\"provider_instance_name\":\"openai-primary\",\"provider_type\":\"cloud_api\",\"provider_profile_id\":\"openai\",\"protocol_adapter_id\":\"openai-responses\",\"base_url\":\"https://api.openai.com/v1\",\"credentials\":{\"api_token\":{\"inline_secret\":\"...\"}},\"provider_rules_id\":\"openai\"}]}"
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
    "value": "{\"provider_instance_name\":\"openai-primary\",\"provider_type\":\"cloud_api\",\"provider_profile_id\":\"openai\",\"protocol_adapter_id\":\"openai-responses\",\"base_url\":\"https://api.openai.com/v1\",\"credentials\":{\"api_token\":{\"inline_secret\":\"...\"}},\"provider_rules_id\":\"openai\"}"
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
  "method": "chat.completions.create",
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
  "method": "chat.completions.create",
  "params": {
    "capability": "llm",
    "model": {
      "alias": "gpt-5.2@openai-primary"
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
  "sys": [3007, "<session_token>", "trace-aicc-exact"]
}
```

精确模型调用从不切换模型或 Provider，失败时返回该 Provider 的真实错误，适合验证单个 Provider。

## 9. 常见错误

- `no_provider_available`：Provider 未注册、`logical_mounts` 不包含目标逻辑目录、Provider 不可用、策略过滤后无候选。
- `logical_model_not_found`：目标逻辑模型不存在或没有可用候选。
- `max_cost_exceeded`：所有候选超过 `requirements.max_cost.amount`；金额币种由 `requirements.max_cost.currency` 明确指定。
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
- 只有管理员层在 system-config：`services/aicc/settings.routing`（见 §2.1）。用户层、路由策略组和 App 绑定存在 AICC RDB（`aicc_routing_layer`、`aicc_routing_group`、`aicc_routing_binding`），由用户本人通过 `routing.usecase.set`（`target: "user"` 或 `"group:<id>"`）、`routing.groups.*`、`routing.bindings.*` 修改，不能用本文的 system-config 接口写。会话绑定存在 lib_opendan 的会话配置（`route_group`），请求时带上。请求本身不再携带 overlay（`session_overlay` 已删除），只能用 `policy` 收紧。
- `models.list` 返回“默认逻辑目录配置 + system_config 中 AICC 系统配置”的合并视图，不包含用户层和策略组；看对某个用户生效的用途表用 `routing.usecases`。
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
