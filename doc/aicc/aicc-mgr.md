# AICC Manager 后端接口设计

状态：Beta 2.2 目标规范

## 1. 目标与边界

AI Center 前端只调用 AICC kRPC，不直接读写 system-config。AICC 管理 API 负责 Provider Instance 的创建、修改、删除、验证、模型 discovery、catalog 查询和 usage 查询。

Provider Instance、Provider Profile 和 Protocol Adapter 是不同身份：

- Instance 是用户配置的具体账号/endpoint，持久化在 `services/aicc/settings`；
- Profile 是渠道 discovery、origin mapping、operation 和价格规则；
- Adapter 是程序已注册的 wire protocol 实现；
- Catalog 只能提供默认 Profile/endpoint/adapter，不能修改实例私有配置。

所有写操作通过 `SystemConfigClient::exec_tx` 做 CAS；完整校验成功后再原子 reload。Beta 2.2 不读取旧 Provider family section、`provider_driver`、section 级 token 或字段别名。

## 2. Beta 2.2 目标配置

### 2.1 AICC 管理方法

- `models.list`
- `provider.catalog`
- `protocol_adapter.list`
- `provider.validate`
- `provider.add` / `provider.update` / `provider.delete`
- `provider.refresh_models`
- `provider.list` / `provider.health`
- `usage.query` / `trace.query`
- `provider_catalog_update.get` / `provider_catalog_update.set`
- `service.reload_settings`

方法不提供 `service.*` 双入口、错误拼写或旧名称兼容别名。

Provider Instance 的库存刷新定时任务是实例级运行时资源。`provider.update` 把实例从 enabled 改为 disabled、更新导致实例重建、`provider.delete`、reload 移除或替换实例，以及 AICC 服务停止时，管理层必须先把实例标记为 stopping，向其定时任务循环发送幂等 `Stop` 事件并等待循环优雅退出，再完成 registry 切换或资源清理。停止后不得接受新的定时刷新，也不得提交迟到的 inventory/health 结果。

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

Provider credential 只存在于统一 Provider Instance 的 locked credentials/credential reference 中。Beta 2.2 不迁移或读取旧 provider family section 和 section 级 token。

### 2.3 Provider Instance 配置

`services/aicc/settings` 使用统一 Provider Instance 数组，不按 Provider 名称建立不同 section：

```json
{
  "providers": [
    {
      "provider_instance_name": "openai-main",
      "provider_type": "cloud_api",
      "provider_profile_id": "openai",
      "protocol_adapter_id": "openai-responses",
      "endpoint": "https://api.openai.com/v1",
      "credentials": {
        "api_token": { "locked": "..." }
      },
      "region": null,
      "provider_rules_id": "openai"
    }
  ],
  "session_config": {}
}
```

字段约束：

- `provider_instance_name` 是 Zone 内稳定唯一主键。
- `provider_type` 只表达部署类型，不表达厂商或协议。
- `provider_profile_id` 必须来自 Known Provider catalog 或 `custom`。
- `protocol_family_id` 只用于 `custom` Provider 的创建/更新请求，表达 OpenAI-compatible、Claude-compatible、Gemini-compatible 等大类；解析成功后可由 resolved Adapter 反查，不作为另一个运行期选择字段。
- `protocol_adapter_id` 是后端解析并保存的内部执行字段，必须来自运行时 adapter registry；Known Provider 由 Profile 给出确定值，`custom` Provider 的创建请求不要求用户填写。
- `custom` Provider 只提交协议族、`endpoint` 和凭据；后端在保存前先测官方新接口，再测该协议族中已注册的历史接口，并固化首个协议验证成功的 Adapter。
- 只有明确的“接口不支持”结果才继续下一候选；连接、认证、限流和服务端故障直接返回，不能用旧接口测试掩盖。
- 凭据使用 system-config locked value 或 credential reference，不进入 catalog、inventory、trace 或日志。
- Catalog 更新不得修改实例名称、endpoint、凭据、区域、账号或协议选择。
- 不读取 `instance_id`、`provider_driver`、`base_url`、`api_key`、`apiKey` 等旧字段或别名。

## 3. 设计原则

1. AICC 服务提供管理 API，前端只调 AICC kRPC，不直接调 system_config。
2. 写操作必须使用 `SystemConfigClient::exec_tx`，并用 `services/aicc/settings` 的 revision 作为 `main_key` 做 CAS。
3. 写成功后默认触发内存 reload，保证 `models.list` 立即反映变更。
4. 返回值优先使用现有 `models.list` 的 raw inventory 模型，UI 继续在 `aicc_mgr.ts` 内做 Raw -> StoreSnapshot 转换。
5. 不引入新的持久依赖。usage 已经使用 AICC RDB，settings 继续使用 system_config。

## 4. kRPC 接口

### 4.0 `provider.catalog` / `protocol_adapter.list`

`provider.catalog` 返回当前 active Known Provider catalog，至少包含 `provider_profile_id`、显示名、默认 `base_url`、内部默认 `protocol_adapter_id` 和 UI hints。Adapter 默认值供后端解析 Known Provider，不要求 UI 暴露 API 版本选择。Catalog 只提供表单默认值，不能覆盖 Provider Instance 私有配置。

`protocol_adapter.list` 返回当前实际注册的 `protocol_family_id`、adapter ID、接口代际/状态、探测优先级、可选 `base_adapter_id`、支持的 operation 和协议能力。每个协议族必须包含官方新接口；历史接口仅在已有具体派生 Provider 需要并完成实现时出现。`sn-openai` 等派生 Adapter 使用独立 ID，并展示其确定的基础 Adapter。该接口用于后端接入解析、诊断和管理员只读展示，不作为普通用户的 API 版本选择列表。

Provider Wizard 每次打开只读取一次完整 catalog；catalog 不可用时仍允许进入手工模式。手工模式让用户选择 OpenAI-compatible、Claude-compatible、Gemini-compatible 等协议族，不要求识别 Responses、Chat Completions、Interactions 等 API 代际。保存前由后端执行 endpoint、认证、协议和 discovery 验证并返回 resolved Adapter。

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
  "provider_type": "cloud_api",
  "provider_profile_id": "openai",
  "protocol_adapter_id": "openai-responses",
  "endpoint": "https://api.openai.com/v1",
  "credentials": {
    "type": "bearer",
    "secret": "sk-..."
  },
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
- Device JWT 等非 API Key Profile 按 Profile 的认证 schema 校验。
- Profile、Adapter 和 endpoint 必须同时通过校验。
- `provider_instance_name` 可选；传入时只用于校验命名合法性，不要求已存在。
- 第一版可以只做参数校验和轻量 HTTP 探测；若能复用已有 provider adapter 的 inventory refresh 逻辑，则返回真实 `models_discovered`。
- 返回错误不应泄露 token、Authorization header 或完整 URL query。

### 4.3 `provider.add`

用途：对应 `AICCMgr.addProvider(draft)`。

Request：

```json
{
  "provider_instance_name": "openai-work",
  "provider_type": "cloud_api",
  "provider_profile_id": "openai",
      "protocol_adapter_id": "openai-responses",
  "endpoint": "https://api.openai.com/v1",
  "credentials": {
    "type": "bearer",
    "secret": "sk-..."
  },
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
2. 校验 Profile、Adapter、endpoint、认证 schema 和 Provider Rules 引用。
3. 校验 request 中的 `provider_instance_name` 非空且全局唯一。
4. 写回 `services/aicc/settings`。
5. 使用 `exec_tx(tx, Some(("services/aicc/settings", version)))`。
6. 调用内部 `handle_reload_settings()`。

`provider_instance_name` 由 UI 生成并传入。下面是 UI 可使用的默认命名基准：

| Provider Profile | 默认 instance name |
| --- | --- |
| `sn` | `sn-ai-provider-main` |
| `openai` | `openai-main` |
| `claude` | `claude-main` |
| `gemini` | `google-gemini-main` |
| `openrouter` | `openrouter-main` |
| `custom` | `custom-<slug(name)>` |

如果默认名已经存在，UI 应追加短随机/递增后缀，例如 `openai-main-2`。后端只做唯一性校验，避免同名 instance 覆盖。

如果 `provider_instance_name` 缺失或同名已存在：

- 缺失时返回 `ReasonError("provider_instance_name is required")`。
- `provider.add` 应返回 `ReasonError("provider already exists")`。
- 后续如需编辑已有 provider，应新增 `provider.update`，不要让 add 混合 upsert 语义。

写入示例：

```json
{
  "providers": [{
    "provider_instance_name": "openai-work",
    "provider_type": "cloud_api",
    "provider_profile_id": "openai",
    "protocol_adapter_id": "openai-responses",
    "endpoint": "https://api.openai.com/v1",
    "credentials": {
      "type": "bearer",
      "secret_ref": "system-config://secrets/aicc/openai-work"
    },
    "timeout_ms": 60000,
    "enabled": true
  }]
}
```

`openrouter`、`sn` 和需要内置扩展的兼容渠道使用独立派生 Adapter；小型 `custom` Provider 由接入测试自动解析其实际支持的 Adapter：

- `provider_profile_id` 标识渠道规则。OpenAI 使用 `openai`，OpenRouter 使用 `openrouter`，自定义 OpenAI-compatible Provider 使用 `custom` 或已注册的 Profile ID。
- `protocol_adapter_id` 必须来自 AICC 运行时注册表。Known Provider catalog 给出确定值；自定义 Provider 由后端按新接口优先顺序解析。UI 只展示解析结果和诊断，不要求用户修正 API 版本。
- `provider_type` 只表示部署类型（例如 `cloud_api`），不能代替 `provider_profile_id`。新配置不读取旧 `provider_driver` 字段。
- 派生 Adapter 必须有独立 ID 和可选 `base_adapter_id`；基础 Adapter 不读取派生 Provider 配置。
- 官方 Profile 默认选择新接口；自定义 Provider 保存时自动测试新接口和已注册历史接口，resolved Adapter 一旦保存，运行时不能从新接口静默降级。

`sn-ai-provider` 使用独立 Profile 和 `sn-openai` Adapter，后者属于 `openai` 协议族并派生自 `openai-responses`。`auth.mode=api_key` 时使用静态 Bearer API Key；`auth.mode=dynamic_login` 时由 SN 层使用登录凭据换取并刷新短期 token，再委托 Responses 实现。OpenAI 官方 Adapter 不包含 SN 登录或 Provider 分支。模型能力仍由 Model Driver catalog 声明，SN discovery 只能收窄可用集合；实际价格优先来自 discovery，无法发现时使用 Provider Rules 中的静态价格。

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
2. 遍历统一 `providers[]`。
3. 校验删除目标及 policy 引用，`exec_tx` CAS 写回不再包含该实例的新 settings。
4. reload 应先把旧实例标记为 stopping，向其库存刷新定时任务循环发送 `Stop` 事件，并等待循环优雅退出。
5. 循环退出后从 registry 删除实例，并同步删除或解除其 credential reference；动态 token 只存在内存，无需持久化清理。
6. 原子发布新 registry；停止过程失败时不得留下仍可路由但 settings 已删除的半状态，必须返回可诊断错误。

禁用实例以及因 endpoint、Profile、Adapter、认证等变化而重建实例时使用相同停止协议：先停止旧实例的定时任务，再发布禁用状态或新实例。AICC 服务停止时应向全部 Provider 定时任务循环广播 `Stop` 并等待退出。

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

AI Center 通过这两个接口配置和观察 NDN metadata 文件更新。AICC 不实现下载校验、activation、LKGS、水位或专用后台生效流程：

- `get` 返回配置状态、NDN 报告的 `metadata_target_seq`，以及各 Provider 的 `metadata_applied_seq`；不包装一套 AICC 更新状态机。
- `set` 只把启停、源和检查周期配置交给 NDN。NDN 负责发现版本、下载、校验、替换文件，并在替换成功后推进目标序列。
- `set.ok` 只表示配置已持久化并交给 NDN，不表示 metadata 已经进入 AICC 运行时。
- 下一次推理前或任一 Provider Instance 定时库存刷新发现 applied/target seq 不一致时，AICC 统一收敛所有落后库存；model 列表未变化且 seq 相同的 Provider 只探测、不重写库存。
- 写操作复用 settings revision CAS 和调用者 token，不允许前端直接写 `system_config`。

`driver_metadata_update.get` Request：

```json
{}
```

Response：

```json
{
  "enabled": true,
  "source_url": "ndn://metadata.example/aicc/driver-metadata",
  "source_configured": true,
  "interval_secs": 900,
  "metadata_target_seq": 42,
  "providers": [
    {
      "provider_instance_name": "openai-main",
      "metadata_applied_seq": 41
    }
  ]
}
```

文件下载和校验诊断直接使用 NDN 状态，不在 AICC API 中重新定义错误分类。

`driver_metadata_update.set` Request：

```json
{
  "enabled": true,
  "source_url": "ndn://metadata.example/aicc/driver-metadata",
  "interval_secs": 900
}
```

`enabled` 必填；`source_url` 和 `interval_secs` 可选。未知字段按请求解析错误拒绝。源格式、检查周期和下载策略由 NDN 契约校验，AICC 不复制这些规则。

Response：

```json
{
  "ok": true,
  "settings_revision": 17,
  "settings": {
    "enabled": true,
    "source_url": "ndn://metadata.example/aicc/driver-metadata",
    "source_configured": true,
    "interval_secs": 900
  },
  "runtime_apply": {
    "ok": true,
    "ndn_configured": true
  }
}
```

Rust 契约统一定义在 `buckyos-api::aicc_client` 的 `DriverMetadataUpdate*` 类型、`AiccClient`、`AiccHandler` 和 `AiccServerHandler` 中；服务端和其它 Rust 调用方不得再手写字段名。

### 4.8 `service.reload_settings`

该方法是唯一 settings reload 入口；不定义 `reload_settings`、`reaload_settings` 或 `service.reaload_settings` 兼容别名。

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

当前 wizard 只有 add/delete/refresh/validate。编辑已有 provider 时再新增 `provider.update`，语义为修改 endpoint、Profile、Adapter、`auth`、discovery 和实例规则等字段。

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
6. 静态 API Key 通过 Provider Instance 的 locked credential 或 credential reference 保存；动态 token 只保存在派生 Adapter 的运行时凭据缓存，不写回 system-config。
7. SN 动态登录、刷新与认证错误必须在 `sn-openai` 层处理，不能进入 OpenAI 基础 Adapter。

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
  - OpenAI、Claude、Gemini 按 API 代际分别提供专门实现和测试；官方 Profile 只默认新接口。
  - 旧接口 Adapter 与新接口 Adapter 平级，只共享协议中立底层组件，不共享 endpoint 分支或 fallback 状态机。
  - SN 使用独立 `sn-openai` Adapter，通过组合、委托或继承复用 `openai-responses`，只在 SN 层实现静态 API Key/动态登录认证。
  - 其他内置兼容渠道采用相同的独立派生 Adapter 语义，不在基础 Adapter 中增加 Provider 分支。
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
5. 连续添加多个同 Profile Provider Instance，确认凭据和 inventory 不互相覆盖。
6. 调 `provider.refresh_models`，确认执行指定 provider 的 inventory refresh，不只是全量 reload。
7. 分别禁用、删除、替换 Provider 以及停止 AICC 服务，确认都向对应库存刷新定时任务循环发送 `Stop`、等待优雅退出，且退出后没有孤儿定时器或迟到的 inventory/health 写入；删除时再确认实例及其 credential reference 被清理。
8. 调 `usage.query`，确认没有 usage db 时返回空 aggregate，有 usage db 时能返回 summary / bucket。
9. 使用无 `services/aicc/settings` 写权限的普通 token 调 `provider.add/delete`，确认被 system_config 拒绝。

风险：

- 本版本改动 Provider settings schema，是明确的 breaking change；实现时必须同步更新所有 parser、默认配置和测试数据。
- 派生 Adapter 若把厂商分支下沉到基础 Adapter，会重新形成不可拆除的历史负担；代码评审和删除性测试必须阻止该情况。
- 动态 token 刷新需要处理并发合并、过期和认证失败，但这些状态不能污染持久 catalog 或 inventory。
- `provider.refresh_models` 需要把现有私有 `refresh_inventory_once` 抽成 trait 能力，涉及所有 Provider Adapter。
