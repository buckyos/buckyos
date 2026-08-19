# AICC Manager 后端接口设计

## 1. 任务确认

本设计面向 `src/frame/desktop/src/api/aicc_mgr.ts` 的真实后端接入需求，在 `src/frame/aicc` 服务中补齐 AI Center 管理页需要的 kRPC 接口。

当前前端已经通过 `buckyos.getServiceRpcClient('aicc')` 调用：

- `models.list`：读取模型目录、Provider inventory、当前 `session_config`。
- `service.reload_settings`：重新加载 `services/aicc/settings` 并刷新 AICC 内存 Provider。

当前前端仍缺少真实后端的能力：

- 添加 Provider。
- 删除 Provider。
- 刷新某个 Provider 的模型列表。
- 校验 Provider 连接。
- 读取 usage summary / trend。

这些接口的写操作应由 AICC 服务封装，内部通过 `SystemConfigClient::exec_tx` 事务更新 `system_config`，前端不直接操作 `system_config`。

本版本允许对 AICC provider settings schema 做 breaking change：Provider credential 从 section 级迁移到 instance 级，以支持同一种 provider family 下配置多个账号 / token，例如 4 个 OpenAI API Key。

## 2. 现有实现约束

### 2.1 AICC 服务现状

入口文件：

- `src/frame/aicc/src/main.rs`
- `src/frame/aicc/src/aicc.rs`

已有管理类方法：

| Method | 状态 | 行为 |
| --- | --- | --- |
| `models.list` | 已实现 | 调用 `AIComputeCenter::dump_model_directory()` 返回 providers / directory / aliases / session_config |
| `service.models.list` | 已实现 | `models.list` 别名 |
| `reload_settings` | 已实现 | 读取 `runtime.get_my_settings()` 并重新注册 providers |
| `service.reload_settings` | 已实现 | `reload_settings` 别名 |

Provider 注册流程：

1. AICC 启动或 reload 时读取 `services/aicc/settings`。
2. `apply_provider_settings()` 清空 registry 和 route。
3. 依次调用 `register_openai_llm_providers`、`register_sn_ai_provider`、`register_google_gemini_providers`、`register_claude_providers`、`register_minimax_providers`、`register_fal_providers`。
4. 应用默认 logical tree。

### 2.2 settings key

当前 AICC 真实运行配置主 key：

```text
services/aicc/settings
```

现有 control_panel 还维护过一组 UI 辅助配置：

```text
services/control_panel/ai_models/policies
services/control_panel/ai_models/provider_overrides
services/control_panel/ai_models/model_catalog
services/control_panel/ai_models/provider_secrets
```

本设计不继续扩大 control_panel 的 AICC 配置面。新的 AI Center 后端接口应以 `services/aicc/settings` 为主真相源。

本版本将 `api_token` / `api_key` 从 provider family section 下沉到 `instances[]` 的每个 instance 中。这是 breaking change，需要同步修改所有 AICC provider parser。旧格式只作为一次性迁移输入，不做长期兼容。

### 2.3 当前 provider section 格式

`services/aicc/settings` 顶层按 provider family 分 section；每个真实 provider instance 都在 section 的 `instances[]` 内保存自己的 endpoint、token 和模型配置：

```json
{
  "openai": {
    "enabled": true,
    "instances": [
      {
        "provider_instance_name": "openai-main",
        "api_token": "...",
        "base_url": "https://api.openai.com/v1"
      }
    ]
  },
  "sn-ai-provider": { "enabled": true, "instances": [] },
  "google": { "enabled": true, "instances": [] },
  "claude": { "enabled": true, "instances": [] },
  "minimax": { "enabled": true, "instances": [] },
  "fal": { "enabled": true, "instances": [] }
}
```

各 section 支持的主要字段：

| Provider type | settings section | instance 字段 |
| --- | --- | --- |
| `sn_router` | `sn-ai-provider` | `provider_instance_name`, `provider_type`, `base_url`, `timeout_ms` |
| `openai` / `openrouter` / `custom` | `openai` | `provider_instance_name`, `provider_type`, `provider_driver`, `api_token`, `base_url`, `timeout_ms` |
| `google` | `google` | `provider_instance_name`, `provider_type`, `provider_driver`, `api_token`, `base_url`, `timeout_ms`, `models`, `default_model`, `image_models`, `default_image_model`, `features`, `alias_map` |
| `anthropic` | `claude` | `provider_instance_name`, `provider_type`, `provider_driver`, `api_token`, `base_url`, `timeout_ms`, `models`, `default_model`, `features`, `alias_map` |
| `minimax` | `minimax` | `provider_instance_name`, `provider_type`, `provider_driver`, `api_token`, `base_url`, `timeout_ms`, `models`, `default_model`, `features`, `alias_map` |
| `fal` | `fal` | `provider_instance_name`, `provider_type`, `api_token`, `base_url`, `timeout_ms`, `image_upscale_models`, `image_bg_remove_models`, `audio_enhance_models`, `video_upscale_models` |

`provider_instance_name` 是 UI 与后端之间的 Provider ID。前端当前也用 `inventory.provider_instance_name` 作为 `ProviderView.config.id`。UI 创建 provider 时必须传入全局唯一的 `provider_instance_name`；后端只提供默认命名建议并做冲突检查。

## 3. 设计原则

1. AICC 服务提供管理 API，前端只调 AICC kRPC，不直接调 system_config。
2. 写操作必须使用 `SystemConfigClient::exec_tx`，并用 `services/aicc/settings` 的 revision 作为 `main_key` 做 CAS。
3. 写成功后默认触发内存 reload，保证 `models.list` 立即反映变更。
4. 返回值优先使用现有 `models.list` 的 raw inventory 模型，UI 继续在 `aicc_mgr.ts` 内做 Raw -> StoreSnapshot 转换。
5. 不引入新的持久依赖。usage 已经使用 AICC RDB，settings 继续使用 system_config。

## 4. kRPC 接口

### 4.1 `models.list`

状态：保留并作为 AI Center 首页 snapshot 的主读接口。

Request 可为空；Routing 页面按面包屑目录加载时可传：

```json
{
  "logical_path": "llm.plan"
}
```

传入 `logical_path` 时，Response 只返回该逻辑路径子树相关的 `directory` / `logical_definitions`，并裁剪 `providers[].models` 到挂载在该路径子树下的模型，避免 Routing 页面为了展示某一层级一次性拉取并组织完整目录。

Request：

```json
{}
```

Response：

```json
{
  "providers": [],
  "directory": {},
  "aliases": [],
  "session_config": {}
}
```

需要增强的字段：

- `providers[].provider_origin`：当前 `dump_model_directory()` 没有输出，前端会 fallback 成 `provider_claimed`。建议补上 `inventory.provider_origin`。
- `providers[].provider_type_revision`：可选，前端 raw 类型已经预留。
- `providers[].models[].pricing` / `attributes`：当前未输出，前端会 fallback 成 unknown。不是添加 Provider 的阻塞项。

### 4.2 `provider.validate`

用途：对应 `AICCMgr.validateConnection(draft)`。

Request：

```json
{
  "provider_instance_name": "openai-work",
  "provider_type": "openai",
  "name": "OpenAI Main",
  "endpoint": "https://api.openai.com/v1",
  "protocol_type": "openai_compatible",
  "api_key": "sk-...",
  "auto_sync_models": true
}
```

Response：

```json
{
  "endpoint_reachable": true,
  "auth_valid": true,
  "models_discovered": ["gpt-4.1-mini", "text-embedding-3-large"],
  "balance_available": false,
  "errors": [],
  "error_details": [
    {
      "kind": "models",
      "message": "model discovery returned no models"
    }
  ]
}
```

实现要求：

- 不写 system_config。
- `sn_router` 允许没有 `api_key`。
- `custom` 必须有 `endpoint`。
- `provider_instance_name` 可选；传入时只用于校验命名合法性，不要求已存在。
- 第一版可以只做参数校验和轻量 HTTP 探测；若能复用已有 provider adapter 的 inventory refresh 逻辑，则返回真实 `models_discovered`。
- 返回错误不应泄露 token、Authorization header 或完整 URL query。

### 4.3 `provider.add`

用途：对应 `AICCMgr.addProvider(draft)`。

Request：

```json
{
  "provider_instance_name": "openai-work",
  "provider_type": "openai",
  "name": "OpenAI Main",
  "endpoint": "https://api.openai.com/v1",
  "protocol_type": "openai_compatible",
  "api_key": "sk-...",
  "auto_sync_models": true
}
```

Response：

```json
{
  "ok": true,
  "provider_instance_name": "openai-work",
  "settings_revision": 13,
  "reload": {
    "ok": true,
    "providers_registered": 2
  }
}
```

事务写入：

1. 读取 `services/aicc/settings`，拿到 `version`。
2. 根据 `provider_type` 定位 section。
3. 校验 request 中的 `provider_instance_name` 非空且全局唯一。
4. 写回 `services/aicc/settings`。
5. 使用 `exec_tx(tx, Some(("services/aicc/settings", version)))`。
6. 调用内部 `handle_reload_settings()`。

`provider_instance_name` 由 UI 生成并传入。下面是 UI 可使用的默认命名基准：

| Provider type | 默认 instance name |
| --- | --- |
| `sn_router` | `sn-ai-provider-main` |
| `openai` | `openai-main` |
| `anthropic` | `claude-main` |
| `google` | `google-gemini-main` |
| `openrouter` | `openrouter-main` |
| `custom` | `custom-<slug(name)>` |

如果默认名已经存在，UI 应追加短随机/递增后缀，例如 `openai-main-2`。后端只做唯一性校验，避免同名 instance 覆盖。

如果 `provider_instance_name` 缺失或同名已存在：

- 缺失时返回 `ReasonError("provider_instance_name is required")`。
- `provider.add` 应返回 `ReasonError("provider already exists")`。
- 后续如需编辑已有 provider，应新增 `provider.update`，不要让 add 混合 upsert 语义。

section 映射：

```text
sn_router  -> sn-ai-provider
openai     -> openai
openrouter -> openai
custom     -> openai
anthropic  -> claude
google     -> google
minimax    -> minimax
```

写入示例：

```json
{
  "openai": {
    "enabled": true,
    "instances": [
      {
        "provider_instance_name": "openai-work",
        "provider_type": "cloud_api",
        "provider_driver": "openai",
        "api_token": "sk-...",
        "base_url": "https://api.openai.com/v1",
        "timeout_ms": 60000
      }
    ]
  }
}
```

`openrouter` 和 `custom` 第一版复用 `openai` adapter：

- OpenAI instance 支持显式配置 `provider_driver`。OpenAI 使用 `openai`，OpenRouter 使用 `openrouter`，自定义 OpenAI-compatible provider 使用与其 driver metadata 文件一致的 driver id。后端 inventory 会原样返回该值，并用它选择对应的模型 metadata。
- `provider_type` 只表示部署类型（例如 `cloud_api`），不能代替 `provider_driver`。未配置 `provider_driver` 时回退为 `openai`；因此 OpenRouter 和 custom instance 应显式配置该字段。

`sn-ai-provider` 使用独立 adapter，不通过 `OpenAIProvider` 注册。它固定使用 `runtime_session` 鉴权，通过 SN 的 `/models` 刷新 inventory，并通过 `/responses` 执行 `llm.chat`；配置中不接受普通 API key。初始模型取独立 `sn-ai-provider` driver metadata 的 `models[].id` 中声明支持 LLM 的项目，随后由 `/models` 返回的真实 inventory 刷新。metadata 未描述的模型按该 metadata 中的最高价格估算。

### 4.4 `provider.delete`

用途：对应 `AICCMgr.deleteProvider(id)`。

Request：

```json
{
  "provider_instance_name": "openai-main"
}
```

Response：

```json
{
  "ok": true,
  "provider_instance_name": "openai-main",
  "settings_revision": 13,
  "reload": {
    "ok": true,
    "providers_registered": 1
  }
}
```

事务写入：

1. 读取 `services/aicc/settings`。
2. 遍历所有已知 provider section 的 `instances[]`。
3. 删除 `provider_instance_name` 匹配的 instance。
4. 删除 instance 时同步删除该 instance 内的 `api_token` / `api_key`；如果 section 的 instances 为空，将 `enabled` 置为 `false`。
5. `exec_tx` CAS 写回。
6. reload。

未找到时返回：

```json
{
  "ok": false,
  "reason": "provider_not_found"
}
```

### 4.5 `provider.refresh_models`

用途：对应 `AICCMgr.refreshProviderModels(id)`。

Request：

```json
{
  "provider_instance_name": "openai-main"
}
```

Response：

```json
{
  "ok": true,
  "provider_instance_name": "openai-main",
  "inventory_revision": "provider-inventory-3-..."
}
```

第一版实现策略：

- 对齐各 adapter 已有的 `refresh_inventory_once` 语义，执行指定 provider 的真实 inventory refresh。
- 需要把 refresh 能力提升到公共接口，例如在 `Provider` trait 增加 `async fn refresh_inventory(&self) -> Result<ProviderInventory, ProviderError>`，并由 OpenAI / Claude / Gemini / MiniMax / Fal / SN provider 实现。
- refresh 成功后将返回的 inventory 写入 `ModelRegistry`，再触发 route 目录刷新。
- 前端随后会 `refresh()`，因此第一版不必在响应里返回完整 inventory。
- 找不到 provider 时返回 `provider_not_found`。

### 4.6 `usage.query`

用途：补齐 `getUsageSummary()` / `getUsageTrend()` 的真实数据来源。

Request 直接复用 `buckyos_api::QueryUsageRequest`：

```json
{
  "time_range": { "kind": "last30d" },
  "filters": {},
  "group_by": ["provider_model"],
  "time_bucket": "day",
  "output_mode": "summary"
}
```

Response 直接复用 `buckyos_api::QueryUsageResponse`：

```json
{
  "total": {
    "total_requests": 10,
    "input_tokens": 1000,
    "output_tokens": 500,
    "total_tokens": 1500,
    "request_units": 0,
    "finance_amount": 0.0123
  },
  "grouped": [],
  "buckets": [],
  "events": []
}
```

实现要求：

- 从 `AIComputeCenter::usage_log_db()` 获取 DB。
- DB 未初始化时返回空 aggregate，不报错，避免首页不可用。
- UI 的 `UsageSummary` 应优先使用 `usage.query` 的 summary / grouped / bucketed 结果；时间范围由前端按浏览器时区换算成 `explicit` 的 `start_time_ms` / `end_time_ms` 传给后端，避免依赖服务端本地时区。
- `Usage Detail` 原始事件必须使用 `output_mode=events` + `limit` + `cursor` 分页加载，不应为了前端分页或统计一次性加载全部事件。
- `Usage Detail` 的 Provider / Model / App 筛选应通过 `provider_instance_names`、`provider_instance_query`、`provider_models`、`provider_model_query`、`caller_app_ids`、`caller_app_query` 下推到后端；数组字段表示多选精确匹配，`*_query` 表示前端输入框的模糊匹配文本。

建议前端调用：

- Summary：`time_range.kind=explicit`, `output_mode=summary`, `group_by=["provider_model"]`。
- Trend：`time_range.kind=explicit`, `time_bucket=day`, `output_mode=summary`。

### 4.7 `driver_metadata_update.get` / `driver_metadata_update.set`

AI Center 通过这两个接口读取并启用或停用 Provider Driver Metadata 云更新。配置仍持久化在 `services/aicc/settings.driver_metadata_update`：

- `get` 返回 `enabled`、脱敏后的 `source_url`、`source_configured`、`interval_secs`，以及 `status`、`active_revision`、`last_attempt_at_ms`、`last_success_at_ms`、`last_error`、`consecutive_failures` 运行状态。
- `set` 接收 `enabled`，并可选接收 `source_url`；启用时校验 HTTPS、固定路径 `/aicc/driver-metadata/index.json`。写入后只切换 metadata source 并刷新现有 Provider inventory，不执行会清空 registry 的全量 Provider reload。
- `set.ok` 表示 settings 已持久化；`runtime_apply.refresh_scheduled=true` 表示 metadata updater 已收到新配置。接口不等待 Provider 的外部模型发现请求，后台刷新结果随后同步到 `settings.status/last_error`。
- 写操作复用 settings revision CAS 和调用者 token，不允许前端直接写 `system_config`。
- 停用或切换源时保留各 source namespace 的 LKGS 和防回滚水位；重新启用同一源仍沿用原有水位。

`driver_metadata_update.get` Request：

```json
{}
```

Response：

```json
{
  "enabled": true,
  "source_url": "https://metadata.example/aicc/driver-metadata/index.json",
  "source_configured": true,
  "interval_secs": 900,
  "status": "healthy",
  "active_revision": 42,
  "last_attempt_at_ms": 1785830400000,
  "last_success_at_ms": 1785830400000,
  "last_error": null,
  "consecutive_failures": 0
}
```

`status` 只允许 `disabled`、`idle`、`updating`、`healthy`、`degraded`、`error`。

`driver_metadata_update.set` Request：

```json
{
  "enabled": true,
  "source_url": "https://metadata.example/aicc/driver-metadata/index.json",
  "interval_secs": 900
}
```

`enabled` 必填；`source_url` 和 `interval_secs` 可选。未知字段按请求解析错误拒绝，`interval_secs` 归一化到 60 至 86400 秒。停用时省略 `source_url` 表示保留当前源。

Response：

```json
{
  "ok": true,
  "settings_revision": 17,
  "settings": {
    "enabled": true,
    "source_url": "https://metadata.example/aicc/driver-metadata/index.json",
    "source_configured": true,
    "interval_secs": 900,
    "status": "updating",
    "consecutive_failures": 0
  },
  "runtime_apply": {
    "ok": true,
    "refresh_scheduled": true
  }
}
```

Rust 契约统一定义在 `buckyos-api::aicc_client` 的 `DriverMetadataUpdate*` 类型、`AiccClient`、`AiccHandler` 和 `AiccServerHandler` 中；服务端和其它 Rust 调用方不得再手写字段名。

### 4.8 `service.reload_settings`

状态：保留。

管理写接口默认在写成功后内部调用 reload。仍保留显式 reload，用于调试和外部工具修改 `services/aicc/settings` 后手动刷新。

## 5. 暂不做的接口

### 5.1 routing session 写接口

`aicc_mgr.ts` 当前只读取 `session_config`，没有写 routing policy 的方法。因此第一版不增加 routing 写接口。

后续如果 Routing 页面需要编辑，应新增：

```text
routing.session.get
routing.session.set
routing.session.patch_node
```

并先让 AICC 启动 / reload 从 `services/aicc/settings.session_config` 加载全局 session config。否则写入 system_config 不会影响当前内存 route。

### 5.2 provider.update

当前 wizard 只有 add/delete/refresh/validate。编辑已有 provider 时再新增 `provider.update`，语义为修改 endpoint、api_key、auto_sync_models、models、default_model 等字段。

## 6. system_config 事务模型

所有 settings 写接口使用同一套流程：

```text
load settings with revision
  -> validate request
  -> build next settings json
  -> exec_tx update services/aicc/settings with main_key revision
  -> reload settings
  -> return result
```

伪代码：

```rust
let config_client = runtime.get_system_config_client().await?;
config_client.set_context(RPCContext {
    token: req.token.clone(),
    ..Default::default()
}).await?;
let current = config_client.get("services/aicc/settings").await;
let (mut settings, version) = match current {
    Ok(value) => (serde_json::from_str::<Value>(&value.value)?, value.version),
    Err(KeyNotFound(_)) => (json!({}), 0),
    Err(err) => return Err(...),
};

mutate_settings(&mut settings)?;

let mut tx = HashMap::new();
tx.insert(
    "services/aicc/settings".to_string(),
    if version == 0 {
        KVAction::Create(serde_json::to_string_pretty(&settings)?)
    } else {
        KVAction::Update(serde_json::to_string_pretty(&settings)?)
    },
);

let main_key = if version == 0 {
    None
} else {
    Some(("services/aicc/settings".to_string(), version))
};
config_client.exec_tx(tx, main_key).await?;
let next = config_client.get("services/aicc/settings").await?;
```

注意：写接口必须使用当前 RPC request 的 token 设置 `SystemConfigClient` context，不能使用 AICC 服务自己的 service token 代写。否则会绕过 system_config 对 `services/aicc/settings` 的 RBAC。

并发冲突：

- 若 `exec_tx` 返回 revision mismatch，AICC 管理接口返回 `ReasonError("settings_conflict")`。
- 前端应提示用户刷新后重试。
- `settings_revision` 来自写后重新读取 `services/aicc/settings` 的 `version`，不能使用 `exec_tx` 返回值；当前 `SystemConfigClient::exec_tx` 不返回新 revision。

## 7. 前端映射建议

`BuckyOSAiccProvider` 写接口替换为：

| 前端方法 | 后端 method |
| --- | --- |
| `fetchSnapshot()` | `models.list` |
| `addProvider(draft)` | `provider.add`，成功后 `refresh()` |
| `deleteProvider(id)` | `provider.delete`，成功后 `refresh()` |
| `refreshProviderModels(id)` | `provider.refresh_models`，成功后 `refresh()` |
| `validateConnection(draft)` | `provider.validate` |
| `getUsageSummary()` | 基于缓存的 `usage.query` 结果 |
| `getUsageTrend()` | 基于缓存的 `usage.query` 结果 |

`provider.add` 不需要直接返回 `ProviderView`。前端可以按现有模式在写成功后调用 `models.list`，再由 `toStoreSnapshot()` 生成最终 UI 状态。

## 8. 权限与安全

1. Provider 写接口要求调用者有 `services/aicc/settings` 写权限。
2. Provider 写接口必须使用 request token 调 system_config；AICC 服务身份只用于 reload 自身 settings，不用于替调用者写配置。
3. `provider.validate` 不落盘，但会使用用户传入 token 访问外部 endpoint，应限制日志脱敏。
4. 所有日志必须复用 `redact_settings_for_log()` 的规则，至少脱敏 `api_token`、`api_key`、`authorization`。
5. `models.list` 不返回明文 API Key。
6. API Key 在本版本存于 `services/aicc/settings.instances[].api_token`。这是 breaking change，所有 provider 注册函数必须改为从 instance 读取 token；旧 section 级 `api_token` 只用于迁移。

## 9. 实现入口建议

主要改动文件：

- `src/frame/aicc/src/main.rs`
  - 增加 method const。
  - 在 `AiccHttpServer::handle_rpc_call()` 中优先 dispatch 管理接口。
  - 新增 provider settings 读写 helper。
- `src/frame/aicc/src/aicc.rs`
  - 在 `Provider` trait 暴露 `refresh_inventory`，并在 registry / model_registry 中提供按 `provider_instance_name` 刷新并 apply inventory 的方法。
  - 暴露 usage query helper，如不方便可先放在 `main.rs` 调用 `usage_log_db()`。
- `src/frame/aicc/src/openai.rs`、`src/frame/aicc/src/claude.rs`、`src/frame/aicc/src/gemini.rs`、`src/frame/aicc/src/minimax.rs`、`src/frame/aicc/src/fal.rs`、`src/frame/aicc/src/sn_ai_provider.rs`
  - settings parser 改为从每个 instance 读取 `api_token` / `api_key`。
  - 旧 section 级 token 只用于一次性迁移到 instance 级。
  - 实现 `Provider::refresh_inventory`，复用现有 `refresh_inventory_once` 逻辑。
- `src/kernel/buckyos-api/src/aicc_client.rs`
  - 可选：补齐 typed client method。若只供 desktop web 直接用 raw kRPC，第一版可以不改。
- `src/frame/desktop/src/api/aicc_mgr.ts`
  - 接入新增 method。
  - `WizardDraft` 或提交 payload 需要带唯一 `provider_instance_name`。
  - usage 需要异步刷新缓存，不能继续用同步空值长期占位。

## 10. 验证计划

最小验证：

```bash
cd src
cargo test -p aicc
uv run buckyos-build.py --skip-web
```

接口级验证：

1. 启动 DV 环境。
2. 调 `models.list`，确认 existing providers 正常返回。
3. 调 `provider.validate`，确认不写 `services/aicc/settings`。
4. 调 `provider.add`，确认 `services/aicc/settings` 发生一次事务更新，随后 `models.list.providers` 出现新 provider。
5. 连续添加多个同 family provider，例如 4 个 `openai`，确认每个 instance 保留独立 `api_token` 且 inventory 不互相覆盖。
6. 调 `provider.refresh_models`，确认执行指定 provider 的 inventory refresh，不只是全量 reload。
7. 调 `provider.delete`，确认 instance 及其 `api_token` 被删除并 reload。
8. 调 `usage.query`，确认没有 usage db 时返回空 aggregate，有 usage db 时能返回 summary / bucket。
9. 使用无 `services/aicc/settings` 写权限的普通 token 调 `provider.add/delete`，确认被 system_config 拒绝。

风险：

- OpenAI-compatible 的 `openrouter` / `custom` 当前复用 `openai` section，inventory 中的 provider type 可能需要前端推断。
- 本版本改动 provider settings schema，是明确的 breaking change；实现时必须同步更新所有 parser、默认配置和测试数据。
- API Key 当前仍存 `services/aicc/settings.instances[]`，不是最终 secret 管理模型。
- `provider.refresh_models` 需要把现有私有 `refresh_inventory_once` 抽成 trait 能力，涉及所有 provider adapter。
