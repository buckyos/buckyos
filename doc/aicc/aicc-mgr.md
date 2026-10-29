# AICC Manager 后端接口设计

状态：Beta 2.2 目标规范

## 1. 目标与边界

AI Center 前端只调用 AICC kRPC，不直接读写 system-config。AICC 管理 API 负责 Provider Instance 的创建、修改、删除、验证、模型 discovery、catalog 查询和 usage 查询。

Provider Instance、Provider Profile 和 Protocol Adapter 是不同身份：

- Instance 是用户配置的具体账号/服务入口，持久化在 `services/aicc/settings`；管理 RPC、UI DataModel 和 settings 统一使用 `base_url`；
- Profile 是渠道 discovery、origin mapping、operation 和价格规则；
- Adapter 是程序已注册的 wire protocol 实现；
- Catalog 只能提供默认 Profile/`base_url`/adapter，不能修改实例私有配置。

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
- `routing.get` / `routing.update` / `routing.preview`
- `events.list`
- `driver_metadata_update.get` / `driver_metadata_update.set`
- `service.reload_settings`

`provider.list` 返回带 `settings_revision` 的强类型 `ProviderInstanceView` 列表，包括 disabled 或尚未加载 runtime 的配置实例。view 中的 auth 只允许包含 `mode`、`credential_kind` 和 `configured` 状态，不得返回 secret 或 credential reference。Provider settings CAS 冲突统一返回 `settings_revision_conflict`，其 `details` 固定为 `expected_revision` 与 `actual_revision`。

管理面只保留 `service.reload_settings`；`buckyos-api::aicc_client` 和全部调用方同步更新，不保留 `reload_settings` 旧接口、`service.*` 双入口、错误拼写或其它旧名称兼容别名。

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

Provider credential 只存在于统一 Provider Instance 的 typed credential source 中。Beta 2.2 不迁移或读取旧 provider family section、section 级 token 或 `{locked: ...}` 包装。

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
      "base_url": "https://api.openai.com/v1",
      "credentials": {
        "api_token": { "inline_secret": "..." }
      },
      "region": null,
      "provider_rules_id": "openai"
    }
  ],
  "session_config": {},
  "public_named_object_base_url": "https://zone.example/ndn"
}
```

字段约束：

- `provider_instance_name` 是 Zone 内稳定唯一主键。
- `provider_type` 只表达部署类型，不表达厂商或协议。
- `provider_profile_id` 必须来自 Known Provider catalog 或 `custom`。
- `public_named_object_base_url` 可选；仅在 Zone 已部署第三方 Provider 可访问、且无需用户鉴权的 HTTPS Named Object 下载入口时配置。它也是 `UrlOnly` 原始 URL 被 Provider 明确拒绝后的重试能力开关：AICC 下载内容、写入带 tenant/user owner 记录的 Named Object，再用该公网入口生成的新 URL 重试。省略时 `UrlOnly` 保留原失败且不重试。`UrlOrBytes` 的 fallback 使用 bytes，不依赖此配置。直接输入的 tenant/user 私有 Named Object 不因此公开：字节兼容协议由 AICC 鉴权读取后传输，URL-only 协议明确失败。
- `protocol_family_id` 表达 OpenAI、Claude、Gemini 等协议族；`custom` Provider 必须同时显式提交该族已开放的 `protocol_adapter_id`。
- Known Provider 的 Adapter 由 Profile 给出确定值；`custom` Provider 在 UI 中明确选择，例如 `openai-chat-completions` 或 `openai-responses`。跨族组合必须拒绝，选择结果固化到 settings。
- 接入验证不自动探测或切换协议，配置错误由选定 Adapter 的验证返回。
- `credentials` 是 `map<string, ProviderCredential>`；`ProviderCredential` 是 `inline_secret|secret_ref|runtime_ref` 枚举，分别表达内嵌敏感值、外部 Secret Store 引用和运行时凭据引用。
- `inline_secret` 当前由 system-config 持久化，必须按敏感值处理；`runtime_ref` 只保存运行时来源，`secret_ref` 在配置 Secret Resolver 前明确拒绝解析。凭据不进入 catalog、inventory、trace 或日志。
- Catalog 更新不得修改实例名称、`base_url`、凭据、区域、账号或协议选择。
- 不读取 `instance_id`、`provider_driver`、`endpoint`、`api_key`、`apiKey` 等旧字段或别名；管理 RPC 和 UI DataModel 同样拒绝配置字段 `endpoint`，`base_url` 是各层统一使用的正式字段。

## 3. 设计原则

1. AICC 服务提供管理 API，前端只调 AICC kRPC，不直接调 system_config。
2. 写操作必须使用 `SystemConfigClient::exec_tx`，并用 `services/aicc/settings` 的 revision 作为 `main_key` 做 CAS。
3. 写成功后默认触发内存 reload，保证 `models.list` 立即反映变更。
4. 返回值优先使用现有 `models.list` 的 raw inventory 模型，UI 继续在 `aicc_mgr.ts` 内做 Raw -> StoreSnapshot 转换。
5. 不引入新的存储后端。settings 继续使用 system-config；usage、execution 恢复、route/audit、session exact-model 历史和 artifact 租户归属共用 AICC 现有平台 RDB instance。

## 4. kRPC 接口

### 4.0 `provider.catalog` / `protocol_adapter.list`

`provider.catalog` 返回当前 active Known Provider catalog，至少包含 `provider_profile_id`、显示名、默认 `base_url`、内部默认 `protocol_adapter_id` 和 UI hints。Adapter 默认值供后端解析 Known Provider，不要求 UI 暴露 API 版本选择。Catalog 只提供表单默认值，不能覆盖 Provider Instance 私有配置。

`protocol_adapter.list` 返回当前实际注册的 `protocol_family_id`、adapter ID、接口代际/状态、探测元数据、可选 `base_adapter_id`、支持的 operation 和协议能力。Custom Provider 下拉框仅列出 `custom_provider_selectable=true` 的 Adapter，直接提交所选 ID，不执行自动探测。`provider.catalog` 的每个条目还返回所选 `provider_rules_id` 的 `model_driver_overrides` 和完整 `rules_catalog`。自定义 Provider 向导的高级设置仅选择 Channel preset rules，不展开别名、用途等规则细节；详情规则页复用同一预设列表导入完整目录。

Provider Wizard 每次打开只读取一次完整 catalog；catalog 不可用时仍允许进入手工模式。手工模式让用户选择 OpenAI-compatible、Claude-compatible、Gemini-compatible 等协议族，不要求识别 Responses、Chat Completions、Interactions 等 API 代际。保存前由后端执行 `base_url`、认证、协议和 discovery 验证并返回 resolved Adapter。

### 4.1 `models.list`

Request 为 `{}`。服务从同一个 runtime snapshot 返回当前库存、Model Driver 完整目录和逻辑树：

```json
{
  "models": [],
  "catalog": {
    "revision": 2,
    "vendors": [{
      "id": "qwen",
      "revision": 2,
      "models": [{
        "id": "qwen3.5-27b",
        "metadata": {"local_deployable": true, "api_types": ["llm"]},
        "providers": []
      }],
      "specs": [{
        "id": "qwen-dense-27b",
        "path": "llm.qwen-dense-27b",
        "direct_only": false,
        "members": [{
          "model_id": "qwen3.5-27b",
          "target": "llm.qwen3-5-27b:thinking",
          "weight": 1.0,
          "active": false
        }]
      }]
    }]
  },
  "directory": {},
  "logical_definitions": [],
  "generation": 1
}
```

示例仅展示部分元数据字段。`models[]` 仍是已物化的 exact model 库存；`catalog.vendors[].models[]` 来自已生效的 Model Driver 文档，包含 defaults 合并后的完整语义，排除 `exclude: true` 的条目，零 Provider 时仍可浏览。有限 pattern 已在 catalog 编译阶段展开。

`providers[]` 按 `(model_driver_id, origin_model_id)` 关联当前库存，以 provider instance 去重；每项为 `{id, local, exact_models}`。仅启用且实际有库存的 Provider 被计入，`local` 严格来自 `provider_type=local_inference`，不根据模型名称、URL 或 profile 推测。这里的可用表示至少有一个 Provider，不表示健康检查、额度或路由约束均通过。没有凭据字段。

规格来自 Driver 的 `specs`，其成员由 LLM metadata 生成；空规格保留。`target` 包含规格选定的 effort。已物化的成员显示逻辑树当前权重（`active: true`），未物化的成员显示建树默认权重 `1.0`（`active: false`）。此权重是规格到模型家族的权重，不是功能目录到规格的权重，也不承诺路由一定可执行。

Model Driver v2 的 exact rule、有限 pattern 和 defaults 新增可选布尔值 `local_deployable`，exact/pattern 值覆盖 defaults，缺省表示尚未声明本地部署能力。本次依据仓库既有权重模型记录为 Qwen 的七个明确权重型号补充 `true`；不按厂商或名称在前端推断。此字段不代表已经安装，也不创建 Provider。

WebUI 的 `fetchModelCatalog()` 只读取此方法的 `catalog`，独立于 usage 查询。页面提供只读部署入口并注明功能尚未开放；本版本没有部署 RPC、下载任务或虚拟成功状态。

### 4.2 `provider.validate`

用途：对应 `AICCMgr.validateConnection(draft)`。

Request：

```json
{
  "provider_instance_name": "openai-work",
  "provider_type": "cloud_api",
  "provider_profile_id": "openai",
  "protocol_adapter_id": "openai-responses",
  "base_url": "https://api.openai.com/v1",
  "credentials": {
    "api_token": { "inline_secret": "sk-..." }
  },
  "auto_sync_models": true
}
```

Response：

```json
{
  "base_url_reachable": true,
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
- Profile、Adapter 和 `base_url` 必须同时通过校验。
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
  "base_url": "https://api.openai.com/v1",
  "credentials": {
    "api_token": { "inline_secret": "sk-..." }
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
2. 校验 Profile、Adapter、`base_url`、认证 schema 和 Provider Rules 引用。
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
- 编辑已有 Provider 必须使用 `provider.update`，不要让 add 混合 upsert 语义。

写入示例：

```json
{
  "providers": [{
    "provider_instance_name": "openai-work",
    "provider_type": "cloud_api",
    "provider_profile_id": "openai",
    "protocol_adapter_id": "openai-responses",
    "base_url": "https://api.openai.com/v1",
    "credentials": {
      "api_token": {
        "inline_secret": "..."
      }
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

### 4.4 `provider.update`

用途：更新已有 Provider Instance。现有 enable/disable 功能以及实例连接配置更新统一通过本接口完成，不允许 `provider.add` 使用 upsert 语义。

Request 使用 `provider_instance_name` 定位实例，其余字段为需要替换的可选字段：

```json
{
  "provider_instance_name": "openai-work",
  "enabled": false,
  "base_url": "https://api.openai.com/v1",
  "credentials": {
    "api_token": { "inline_secret": "sk-..." }
  }
}
```

接口可以更新 `enabled`、`base_url`、credential、Profile、Adapter、discovery 和实例规则。更新流程必须：

1. 读取当前 settings 和 revision，确认目标实例存在。
2. 合并 patch 后完整校验 Profile、Adapter、`base_url`、认证 schema 和 Provider Rules 引用；未知字段拒绝。
3. 使用当前 RPC 调用者 token 执行 `exec_tx` revision CAS。
4. 禁用实例或连接、Profile、Adapter、认证发生变化时，先把旧实例标记为 stopping，停止并等待其定时刷新任务退出，再发布禁用状态或替换后的实例。
5. 构建并原子发布完整 RuntimeSnapshot；迟到的旧 inventory/health 结果不得写入新 generation。

Response 与 `provider.add` 一样返回 `ok`、`provider_instance_name`、写后重新读取的 `settings_revision` 和 reload 结果。前端已有 enable/disable、修改凭据等操作可以保留自己的调用封装，内部统一调用 `provider.update`。

### 4.5 `provider.delete`

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

禁用实例以及因 `base_url`、Profile、Adapter、认证等变化而重建实例时使用相同停止协议：先停止旧实例的定时任务，再发布禁用状态或新实例。AICC 服务停止时应向全部 Provider 定时任务循环广播 `Stop` 并等待退出。

未找到时返回：

```json
{
  "ok": false,
  "reason": "provider_not_found"
}
```

### 4.6 `provider.refresh_models`

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

### 4.7 `usage.query`

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
    "consumed_request_units": 10,
    "finance_totals": [
      { "amount": 0.0123, "currency": "USD" }
    ],
    "finance_complete": true
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

### 4.8 `driver_metadata_update.get` / `driver_metadata_update.set`

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

### 4.9 `service.reload_settings`

`service.reload_settings` 是唯一的 settings reload 入口。`buckyos-api::aicc_client` 直接更新为调用该 method；删除 `reload_settings` 旧接口以及 `reaload_settings`、`service.reaload_settings` 等错误拼写。

状态：保留。

管理写接口默认在写成功后内部调用 reload。仍保留显式 reload，用于调试和外部工具修改 `services/aicc/settings` 后手动刷新。

## 5. Routing 配置管理

### 5.1 `routing.get` / `routing.update`

`routing.get` 返回当前 settings revision、完整 `AiccRouteOverlay`，以及当前 Registry 对每条路由调整命令的应用结果 `command_status`（`command`、`matched_items`、可选 `stale_reason`）。`routing.update` 使用调用方读取到的 revision 做 CAS，只替换请求中出现的字段：`provider_weights` 和/或 `routing_commands`，其它系统级 routing 字段保持不变；两者都缺省时返回 `invalid_request`。settings 中的内部字段名 `session_config` 不表示 AICC 保存应用 session overlay；请求级 `session_overlay` 始终由调用方传入。

`routing.update` request：

```json
{
  "settings_revision": 12,
  "provider_weights": {
    "openai-main": 1.5
  },
  "routing_commands": [
    {"kind": "vendor_factor", "vendor": "claude", "factor": 1.5},
    {"kind": "spec_factor", "spec": "llm.gpt-pro", "factor": 0.8},
    {"kind": "model_factor", "vendor": "openai", "model": "gpt-5.6-sol", "factor": 2.0},
    {"kind": "item_weight", "path": "llm.chat", "item": "gpt-pro", "weight": 3.0}
  ]
}
```

`provider_weights` 与 `routing_commands` 都采用完整替换语义，不是 patch；空 map/空数组表示清空。revision 不匹配时返回 `settings_revision_conflict`，并在 `details.expected_revision`、`details.actual_revision` 中返回冲突双方版本。

#### 路由调整命令

逻辑目录树会随 metadata、inventory 刷新自动重建，因此 UI 的路由调整不直接改写目录，而是保存为 `AiccRouteOverlay.routing_commands`。Registry 每次构建（所有 overlay 层合并之后、校验之前）都会重新应用这些命令：

- `vendor_factor` / `spec_factor` / `model_factor`：权重系数，默认 1.0，与匹配 item 的权重相乘；多条系数可叠乘。item 的目标是规格目录（由规格所属 model driver 判定厂商）、或其可达 exact model 全部属于该厂商/模型时匹配。只有同一父目录下的相对权重有意义，因此对同一厂商的全部兄弟项一起乘以系数不会改变它们之间的选择。
- `item_weight`：手工模式，直接把 `path` 目录下名为 `item` 的子项权重设为 `weight`，在全部系数之后应用，因此覆盖系数结果。
- 命令引用的对象（厂商、规格、模型、目录或子项）在当前目录树中不存在时，命令不生效也不导致构建失败，`command_status.stale_reason` 给出原因，并产生 `routing_command_stale` 事件；对象恢复后自动重新生效并产生 `routing_command_recovered` 事件。
- 被命令修改的 item 的 `weight_source` 为 `routing_command`，`default_weight` 保持原默认值。

### 5.2 `routing.preview`

对逻辑目录执行与真实请求相同的两阶段路由（不调用 Provider、不记录 session 历史），用于 UI 观察和测试当前路由配置。request：`{"paths": [...], "explain": false}`，`paths` 为空时预览全部逻辑目录。response 为 `settings_revision` 和 `entries`，每项包含 `path`、`api_type`、`kind`（`task` 用途目录 / `spec` 规格 / `family` 模型家族 / `directory` 其它目录）、`available`、`selected_exact_model`、`error`；`explain=true` 时额外返回完整路由 trace（`logical_expansion`、`ranked_candidates`、`filtered_candidates`、`fallback_chain`、`scheduler_profile` 等）。`models.list` 的 `logical_definitions` 同样带 `kind`。

### 5.3 `events.list`

返回 AICC 值得关注的系统事件，供 AI Center 首页展示。request：`{"limit": 50}`（上限 200）。response：`events`（最新在前的事件历史，内存环形缓冲，最多 200 条，服务重启后清空）和 `active_warnings`（当前仍未解决的告警，例如失效的路由调整命令）。事件字段：`event_id`、`created_at_ms`、`level`（`info`/`warning`/`error`）、`kind`、`message`（英文兜底文案，UI 按 `kind` 本地化）、`details`。当前 `kind`：`routing_command_stale`、`routing_command_recovered`、`routing_commands_updated`、`registry_build_failed`、`registry_recovered`。

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
| `previewProviderRules(provider, catalog, modelIds)` | `provider.preview_rules` |
| `getUsageSummary()` | 基于缓存的 `usage.query` 结果 |
| `getUsageTrend()` | 基于缓存的 `usage.query` 结果 |

`provider.add` 不需要直接返回 `ProviderView`。前端可以按现有模式在写成功后调用 `models.list`，再由 `toStoreSnapshot()` 生成最终 UI 状态。

## 8. 权限与安全

1. Provider 写接口要求调用者有 `services/aicc/settings` 写权限。
2. Provider 写接口必须使用 request token 调 system_config；AICC 服务身份只用于 reload 自身 settings，不用于替调用者写配置。
3. `provider.validate` 不落盘，但会使用用户传入 token 访问外部 endpoint，应限制日志脱敏。
4. 所有日志必须复用 `redact_settings_for_log()` 的规则，至少脱敏 `api_token`、`api_key`、`authorization`。
5. `models.list` 不返回明文 API Key。
6. 静态 API Key 使用 Provider Instance 的 `inline_secret`；外部 Secret Store 使用 `secret_ref`；动态登录使用 `runtime_ref`，换取的动态 token 只保存在派生 Adapter 的运行时凭据缓存，不写回 system-config。
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
  - 旧接口 Adapter 与新接口 Adapter 平级，只共享协议中立底层组件，不共享 endpoint 分支或 fallback 状态机；每个历史 API 代际只实现一份共享 Adapter。
  - SN 使用独立 `sn-openai` Adapter，通过组合、委托或继承复用 `openai-responses`，只在 SN 层实现静态 API Key/动态登录认证。
  - 其他内置兼容渠道可以与 SN 一样建立差异层，但多个派生 Adapter 应复用同一个基础/历史 Adapter，不在基础 Adapter 中增加 Provider 分支，也不复制 wire protocol。
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

## Custom Provider 实例规则编辑

PC 的自定义 Provider 详情新增“模型规则”页；手机不展示或挂载编辑器。页面以 provider.list 的 discovered_model_ids 展示去重的发现模型，包括被 exclude_models 禁用、未识别或能力被滤除的条目，按启用优先、模型名升序排列，支持名称包含检索和每页 10/20/50/100 条。分页、检索使用同一次快照，不逐模型请求 RPC。

ProviderInstanceRules.catalog 保存完整的 Provider Rules catalog，仅允许用于 custom。格式复用 [Provider Profile schema](provider_profile_schema.md)：协议映射为 models / patterns 的 operations，价格为 model_pricing。思考档位沿用现有 variants 数组；逐模型修改通过 match.provider_model_id 指定端点模型，不导出 model_variants。修改某模型时，将其从共享规则的匹配范围排除，再加入该模型启用的档位规则；全部关闭时不加入新规则，其他模型的匹配和参数保持原样。别名使用 model_driver_overrides；用途仍为 server_default_routes，表示基础模型使用上游默认思考参数参与明确的 LLM task。

编译时为每个实例生成独立规则索引，复用现有 schema、条件编译和引用校验，不修改共享预制 rules。实例目录价格优先于发现接口价格，没有目录价格时沿用发现价格。管理面额外返回不含凭据的 rules_catalog 和逐模型 resolved_model_rules。rules_catalog 仅返回该实例保存的目录，或实例显式绑定的 provider_rules_id 对应目录；未绑定规则时为空，不能根据协议族补入内置目录。resolved_model_rules 从实例模型 inventory 读取实际操作和价格，思考档位仅返回该实例已发布的档位映射；未配置且未发现的价格为空。页面使用这些实例值展示有效结果，接口默认值标注为默认，不写成显式覆盖；导出/保存仅包含实例的配置规则，不导入无关模型、默认映射或内置参考价。首次保存后，该快照成为实例的显式目录，后续预制目录更新不会覆盖它。

进入编辑时冻结本地草稿；“结束编辑并生效”通过已有 provider.update 提交完整 instance_rules，携带 settings revision 做 CAS。成功完成校验、重新加载和回读后退出编辑；失败保留草稿和错误。取消不提交。切换详情页内其他标签保留编辑器草稿。

预览、复制和导入/导出统一使用现有 `*.provider.json` 目录格式，不增加外层格式。导出文件名为 `<provider_instance_name>.provider.json`，示例：

    {
      "format": "buckyos.aicc.provider-rules-catalog",
      "schema_version": 1,
      "schema_revision": 1,
      "revision_seq": 1,
      "provider_profile_id": "custom",
      "model_driver_overrides": {},
      "server_default_routes": {},
      "models": [],
      "patterns": [],
      "variants": [],
      "model_pricing": []
    }

用途列表写入目录的 `server_default_routes`，按端点模型 ID 映射到明确的 LLM 任务数组；Rust 库存构建和调用降级均读取该配置。实例显式用途配置优先于目录。模型启停沿用 `models[].exclude`，实例别名合并到 `model_driver_overrides`。除 JSON 文件外，也可在编辑模式选择与向导相同的 Channel preset rules 并点击“导入内置规则”，以替换当前草稿；导入期间不切换实例的连接参数或 adapter。可直接导入现有内置 `*.provider.json`，没有声明的可选数组按空值处理；条件规则、请求规则等未编辑字段完整保留。

文件仅包含 Provider 规则，不含实例身份、URL、凭据、API Key 或实例地域策略。导入保留当前实例的连接和地域设置，上限 5 MB，只更新草稿；正式生效前仍执行相同服务端校验。顶部“模型列表 / 完整配置 JSON”切换展示模型摘要或整份规则目录，不再提供逐列预览。编辑中 JSON 展示完整草稿，包含无效字段时显示错误，不展示过期结果。

本地 inventory 缓存同时保存实例规则指纹；修改实例规则后发生发现失败时，不允许回退到旧规则构造的模型目录。规则未变的既有缓存回退仍可使用。

Provider 实例被停用或未加载时，管理快照可从该实例最近一次 inventory 缓存读取已发现 ID，供规则页继续编辑；不会用缓存中的旧操作、档位或价格替代当前配置，也不会自动启动 Provider 或调用上游。


### 结构化模型参数编辑

列表的用途、协议映射和思考档位以最多两项的紧凑列表展示，更多项从右侧固定的“配置参数”按钮进入单模型侧栏查看。宽面板使用表格，桌面窗口内可用宽度不足时按容器宽度改为模型摘要卡片，确保用途、协议、档位和价格均可见，不要求横向滚动才能看到参数。侧栏修改保留在页面草稿，返回列表不触发生效；页面级“结束编辑并生效”仍统一保存，取消丢弃整轮草稿。手机继续不挂载此模块。

编辑选项复用 models.list 的内置模型 metadata 与 logical_definitions，以及 protocol_adapter.list 的导出接口和 default_operations，按需批量查询两次，不逐模型查询。用途仅允许 kind=task 且 api_type=llm 的定义；direct-only 模型不允许绕过规格准入。内置模型按 alias 或实例解析出的 model_driver/origin_model_id 匹配，读取 api_types 与 llm.supported_efforts；无法识别时先修正别名。查询 metadata 仅获取能力范围，不导入厂商价格或 provider_options 模板。

ProtocolAdapterView.default_operations 按运行时相同的操作解析器返回每个 api-type 的默认接口；缺少可确定默认值时省略该项，由服务端解析或要求显式配置。接口下拉只允许当前 adapter 对所选 api-type 的绑定。未设置覆盖时使用默认接口，清除覆盖不禁用能力；能力禁用写入 remove_api_types。高级 request_rules 按 api_type 维护 defaults/set/remove/when，编辑其中一个作用域时保留全局及其他 api_type 的规则；模型整体规则也可通过高级 JSON 编辑。

自定义 Provider 不再按协议族自动补思考档位。只有显式绑定的规则或实例目录中存在对应 effort 的 provider_options 映射，才发布该思考变体；直接调用未配置的变体同样失败。侧栏按 supported_efforts 列出开关，显式空对象也是已配置；取消后在标准 variants 中移除该模型对应档位，并用 match 排除共享规则对该模型的继承。native 继续表示基础模型原生行为，不产生 reasoning 变体。


导入规则草稿后，页面调用 `provider.preview_rules`，提交当前实例名、完整 rules_catalog 和发现模型 ID 集合。该管理 RPC 要求 settings 写权限，使用现有 Rust 目录编译器与运行时相同的模型身份、精确/通配规则、思考档位和价格解析器，批量返回 resolved_model_rules；不修改配置、缓存或运行时，不调用上游。JSON 文件和内置预设导入共用此流程，预览解析失败时保留原草稿。原始 JSON 仍保留通配规则，不展开写成每模型覆盖。未知原厂模型仍能展示匹配的协议规则，但不伪造身份、思考档位、用途或价格。


Provider 文件字段保持原 schema 的名称、值类型及嵌套结构：operations 留在 models/patterns，request_rules 保留 when/defaults/set/remove/canonical_fields，能力移除使用 remove_api_types/remove_features，思考映射使用 variants，价格使用 model_pricing[].pricing。价格数值不换算为每百万 token，零价格和复杂阶梯、时段、条件规则原样保留。导入时只作结构检查，最终由同一 Rust schema 编译校验；revision_seq 超出 JavaScript 安全整数范围时拒绝导入，避免静默改值。

管理快照返回保存的原始实例规则目录，未保存时返回显式绑定的预设；保留 provider_profile_id、custom_provider_adapters 和 schema/revision 字段，不把文件标识重写成 custom 或清空 adapter 声明。实例的 provider_profile_id 和所选 protocol_adapter_id 继续由实例连接设置管理。实例编译器内部隔离规则 ID 和 adapter 声明，不影响文件回读。没有显式用途配置时，不自动补写 server_default_routes 空对象；该字段是本次新增的可选目录字段，并非 operations 的另一种名称。回归对 15 份内置 *.provider.json 的全部原有字段逐项检查导入导出后的 JSON 值与结构。
