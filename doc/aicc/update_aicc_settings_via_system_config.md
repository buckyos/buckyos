# 通过 system_config 更新 AICC 配置

## 1. 目的

本文记录如何把 AICC 的模型入口等配置写入 `system_config`，并让 AICC 在线生效。

## 2. AICC 配置存储位置

AICC 作为 `KernelService`，其 settings 路径为：

- `services/aicc/settings`

说明：AICC 在启动和 `reload_settings` 时会读取该路径内容并重建 provider 注册。

## 3. 更新方式总览

可以通过 `system_config` 的 kRPC 方法更新该 key：

- 全量覆盖：`sys_config_set`
- 局部更新：`sys_config_set_by_json_path`
- 事务更新：`sys_config_exec_tx`（可选）

更新后调用 AICC 的 `reload_settings` 使变更生效。

## 4. 步骤（推荐）

1. 读取当前配置（可选，便于备份）
2. 写入新配置（全量或局部）
3. 调用 AICC `reload_settings`
4. 用一次 `complete` 做联调验证

## 5. kRPC 请求示例

下列示例均为 `POST /kapi/system_config`，请求体使用 kRPC 结构（`method/params/sys`）。

### 5.1 读取当前 AICC 配置

```json
{
  "method": "sys_config_get",
  "params": {
    "key": "services/aicc/settings"
  },
  "sys": [3001, "<session_token>", "trace-aicc-cfg-get"]
}
```

### 5.2 全量覆盖写入（sys_config_set）

注意：`value` 是字符串类型，内部 JSON 需要序列化后再传。

```json
{
  "method": "sys_config_set",
  "params": {
    "key": "services/aicc/settings",
    "value": "{\"openai\":{\"enabled\":true,\"api_token\":\"sk-xxx\",\"instances\":[{\"instance_id\":\"openai-user-1\",\"provider_type\":\"openai\",\"base_url\":\"https://api.openai.com/v1\",\"models\":[\"gpt-5.4\"],\"default_model\":\"gpt-5.4\"}]}}"
  },
  "sys": [3002, "<session_token>", "trace-aicc-cfg-set"]
}
```

### 5.3 局部更新（sys_config_set_by_json_path）

示例：只更新 `/openai` 节点。

```json
{
  "method": "sys_config_set_by_json_path",
  "params": {
    "key": "services/aicc/settings",
    "json_path": "/openai",
    "value": "{\"enabled\":true,\"api_token\":\"sk-xxx\",\"instances\":[{\"instance_id\":\"openai-user-1\",\"provider_type\":\"openai\",\"base_url\":\"https://api.openai.com/v1\",\"models\":[\"gpt-5.4\"],\"default_model\":\"gpt-5.4\"}]}"
  },
  "sys": [3003, "<session_token>", "trace-aicc-cfg-patch"]
}
```

## 6. 触发生效（AICC 热重载）

`POST /kapi/aicc`：

```json
{
  "method": "reload_settings",
  "params": {},
  "sys": [3004, "<session_token>", "trace-aicc-reload"]
}
```

成功时通常返回：

```json
{
  "result": {
    "ok": true,
    "providers_registered": 1
  },
  "sys": [3004, "trace-aicc-reload"]
}
```

## 7. 验证建议

1. 先用 `model.alias = llm.plan.default` 发一条最小 `complete` 请求。
2. 若失败，优先检查：
- `no_provider_available`：模型映射或 provider 未注册成功
- `provider_start_failed`：上游模型接口调用失败
- `resource_invalid`：请求资源/格式不合法

## 8. 注意事项

- `sys_config_set` / `set_by_json_path` 都受 RBAC 控制，token 需有对应 key 写权限。
- `value` 参数是字符串，不是对象；调用端要先做 JSON 序列化。
- 建议先 `sys_config_get` 备份旧值，再变更。
- 变更后务必调用 AICC `reload_settings`，否则不会立即生效。

## 9. 参考代码

- `src/kernel/buckyos-api/src/runtime.rs`
- `src/kernel/buckyos-api/src/system_config.rs`
- `src/frame/aicc/src/main.rs`
- `src/frame/aicc/src/openai.rs`
- `src/frame/aicc/src/gimini.rs`
- `src/frame/aicc/src/minimax.rs`
