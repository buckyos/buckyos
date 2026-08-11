# AICC 新增 Provider 开发指南

本文面向要接入新模型厂商或新部署形态的开发者。新版 AICC 路由以 `doc/aicc/aicc_router.md` 为准：Provider 不再只向 `ModelCatalog` 注册平铺 alias，而是通过 `ProviderInventory` 声明实体模型、精确模型名、API 类型、逻辑挂载点和动态状态；AICC 再用 `ModelRegistry + SessionConfig + Scheduler` 完成路由。

## 1. Provider 接入边界

Provider 代码主要落在 `src/frame/aicc/src/`。每个 Provider 需要实现 `Provider` trait：

- `inventory()`：返回 `ProviderInventory`
- `estimate_cost()`：返回动态成本、延迟、配额状态等估算
- `start()`：实际调用上游模型 API
- `cancel()`：可取消任务时实现；不支持时可返回 `Ok(())`

一个 Provider instance 是可独立调度和计费的实例，不等同于厂商。关键字段：

- `provider_instance_name`：实例唯一名，例如 `openai-primary`
- `provider_type`：可信部署类型，例如 `cloud_api`、`local_inference`、`proxy_unknown`
- `provider_driver`：厂商或适配器名，例如 `openai`、`claude`、`google-gemini`、`minimax`
- `models[]`：该实例声明的实体模型清单

每个模型必须能形成精确模型名：

```text
<provider_model_id>@<provider_instance_name>
```

例如：

```text
gpt-5.2@openai-primary
claude-sonnet-4.5@claude-main
```

## 2. 最小改动面

新增一个 Provider 通常涉及：

- 新增 Provider 模块：`src/frame/aicc/src/<provider>.rs`
- 如有协议转换复杂度，新增：`src/frame/aicc/src/<provider>_protocol.rs`
- 导出模块：`src/frame/aicc/src/lib.rs`
- 接入服务启动注册：`src/frame/aicc/src/main.rs` 的 `apply_provider_settings()`
- Provider settings 解析、实例构建、inventory 构建
- Provider 协议/错误分类测试：`src/frame/aicc/tests/adapter_protocol_tests.rs` 或 Provider 模块内单测
- 如需控制面板配置，接入 `src/frame/control_panel/src/aicc_settings.rs` 和相关 web API/UI
- 如需开箱默认配置，更新 `src/kernel/scheduler/src/system_config_builder.rs`

## 3. 实现步骤

### 步骤 1：定义 settings 与 Provider 结构

建议参考 `openai.rs`、`claude.rs`、`gemini.rs`、`minimax.rs`、`fal.rs`。

实例配置建议包含：

```rust
provider_instance_name
provider_type
provider_driver
base_url
timeout_ms
models
default_model
features
```

Provider 结构通常包含：

```rust
instance: ProviderInstance
inventory: ProviderInventory
client: reqwest::Client
api_token / api_key
base_url
```

Settings 解析建议：

- 支持 `enabled=false` 时返回 `Ok(0)` 或 `Ok(None)`
- 支持 `api_key` / `api_token` 兼容别名
- 支持 `instance_id` 作为 `provider_instance_name` 的旧字段兼容，但新文档和新配置统一写 `provider_instance_name`
- 对空模型名、重复模型名做清洗
- 不要把厂商名写进 `provider_type`；厂商名放 `provider_driver`

### 步骤 2：构建 ProviderInventory

`ProviderInventory` 是新版路由的核心输入。每个模型应声明：

- `provider_model_id`
- `exact_model`
- `api_types`
- `logical_mounts`
- `capabilities`
- `attributes`
- `pricing`
- `health`

示意：

```rust
provider_model_metadata(
    provider_instance_name,
    provider_model_id,
    vec![ApiType::LlmChat],
    llm_logical_mounts(provider_driver, provider_model_id),
)
```

`logical_mounts` 应表达模型可挂载到哪些逻辑目录，例如：

- `llm.chat`
- `llm.plan`
- `llm.gpt5`
- `llm.claude`
- `image.txt2img.gemini`

AICC 会把多个 Provider 的 inventory 汇入 `ModelRegistry`，同一个逻辑模型名可以产生多个候选。

> **能力 metadata 优先走 driver metadata resolver，而不是 Provider 自声明。** Provider 自发现只需负责发现 `provider_model_id`（例如 OpenAI 通过 `/models` 报告模型列表）；`metadata_resolver.rs` 再按 driver metadata（`openai.json` / `claude.json` / `gemini.json` 对应的 gemini / `fal.json` / `minimax.json`）把 model id 转成最终 `ModelMetadata` 的 `capabilities` / `logical_mounts` / `variants`。匹配优先级：exact `models` → `patterns` → `defaults` → conservative fallback；override 链：builtin → remote cache → local override → system-config override。unknown model 保守对待，不默认声明 `tool_call` / `web_search` / `vision` / `json_schema`。schema 见 `doc/aicc/driver_metadata_schema.md`。新接入 Provider 应把按模型名细分能力的规则收编到 driver metadata（参考 `claude.rs` 的 classifier 收编路径），而不是写死在 adapter 里，也不再依赖 legacy `ProviderInstance.features`。
>
> reasoning effort 等档位用 driver metadata 的 `variants` 表达（展开成 `gpt-5.1:reasoning-high@instance` 这类带 variant 的精确模型 + 预置 `provider_options`），不要做成普通请求参数。

### 步骤 3：实现协议转换层

如果上游 API 与 AICC 请求差异较大，建议拆出 `<provider>_protocol.rs`：

- 把 content-block 形态的 `messages`（`AiMessage { role: AiRole, content: Vec<AiContent> }`，见 `aicc_api设计.md` §3.2）转成上游请求；`tool`/`developer` 等 IR role、`tool_use`/`tool_result`/`thinking`/`provider_state` 等 block 在 lowering 时改写为各 provider 原生形态
- 处理 `tools`、`tool_choice`、`response_format`、`max_tokens` 等参数白名单
- 把带 variant 的精确模型还原成 provider base model + provider options（如 `reasoning.effort`）后再发请求
- 对不合法请求返回 fatal 类错误
- 将上游响应转回 content-block `AiMessage`（typed inference 路径）或 `AiResponseSummary`（legacy 路径）

协议层只做结构转换，不做路由决策。

### 步骤 4：实现 `start()` 和错误分类

`start()` 按 `api_type` 或 method 分发到具体调用：

- LLM：`llm.chat` / `llm.completion`
- 图像：`image.txt2img` / `image.img2img`
- 视觉、音频、视频等按 Provider 能力支持

错误分类要稳定：

- HTTP 429、5xx、网络错误、超时：通常标记 `retryable`
- 认证失败、参数错误、协议错误、响应解析错误：通常标记 `fatal`
- 上游返回的配额耗尽应反映到 route trace 或 inventory health，避免后续继续命中同一异常候选

同步完成时返回：

```rust
ProviderStartResult::Immediate(AiResponseSummary { ... })
```

建议把脱敏后的上游请求/响应摘要放到 `extra.provider_io`，便于排障。

### 步骤 5：注册 Provider 和 inventory

新增 `register_<provider>_providers(center, settings)`：

1. 解析 settings
2. 构建 Provider instance
3. `center.registry().add_provider(provider)`
4. 将返回的 `ProviderInventory` 写入 `center.model_registry().write()?.apply_inventory(inventory)`

示意：

```rust
let inventory = center.registry().add_provider(provider);
center
    .model_registry()
    .write()
    .map_err(|_| anyhow!("model registry lock poisoned"))?
    .apply_inventory(inventory)?;
```

`src/frame/aicc/src/main.rs` 的 `apply_provider_settings()` 会在 reload 时清空注册表并重建，因此新增 Provider 必须接入这个入口。

### 步骤 6：接入默认逻辑目录

默认逻辑目录由 `default_logical_tree` 提供，并会先与 `services/aicc/settings.routing_config` 中的系统配置合并，再作为 request/session overlay 的 base。Provider 不应在代码里硬编码“唯一默认模型”，而应通过 `logical_mounts` 声明自己可作为哪些逻辑目录的候选。

如果需要额外的逻辑目录，请同步检查：

- `doc/aicc/aicc_router.md`
- `doc/aicc/aicc 逻辑模型目录.md`
- `src/frame/aicc/src/default_logical_tree.rs`

### 步骤 7：保留 legacy alias 兼容层

当前代码中仍有 `ModelCatalog` 和 `alias_map` 兼容逻辑。新增 Provider 可以保留默认 alias 或自定义 alias 以兼容旧调用，但新接入不应只依赖 alias。

优先级建议：

1. Provider inventory 的 `logical_mounts`
2. 默认逻辑目录配置与 `system_config` 系统配置合并后的 `models.list.logical_definitions` / `models.list.directory`
3. request 级 `session_overlay`
4. legacy `ModelCatalog` alias 兼容层

## 4. Settings 模板

最小可用模板：

```json
{
  "myprovider": {
    "enabled": true,
    "api_token": "YOUR_TOKEN",
    "instances": [
      {
        "provider_instance_name": "myprovider-primary",
        "provider_type": "cloud_api",
        "provider_driver": "myprovider",
        "base_url": "https://api.example.com/v1",
        "timeout_ms": 60000,
        "models": ["model-a", "model-b"],
        "default_model": "model-a",
        "features": ["plan", "json_output", "tool_calling", "web_search"]
      }
    ]
  }
}
```

本地推理 Provider 应使用：

```json
{
  "provider_type": "local_inference"
}
```

不确定实际部署边界的代理服务应使用：

```json
{
  "provider_type": "proxy_unknown"
}
```

`local_only` 策略只应信任 AICC settings 中的 `provider_type`，不能只信任 Provider 自己在 inventory attributes 里的声明。

## 5. 控制面板接入

如果希望用户在控制面板中查看、编辑、诊断 Provider，需要同步接入：

- Provider 卡片展示
- 保存 `services/aicc/settings`
- `ai.provider.test`
- `ai.reload`
- `ai.provider.list`

保存逻辑应写回：

- `enabled`
- `api_token` / `api_key`
- `instances[0].provider_instance_name`
- `instances[0].provider_type`
- `instances[0].provider_driver`
- `instances[0].base_url`
- `instances[0].models`
- `instances[0].default_model`

保存后调用 `service.reload_settings` 或 `reload_settings`，并用 `models.list` 验证 inventory 已更新。

## 6. 测试要求

至少覆盖：

- settings 解析：enabled、token 缺失、默认 instance、字段别名
- inventory 构建：精确模型名、`api_types`、`logical_mounts`、去重
- 成本估算：cost、latency、quota state
- 协议转换：最小成功请求
- 错误分类：429/5xx/network 为 retryable，4xx 参数错误为 fatal
- 路由验证：`model.alias=llm.chat` 或目标逻辑目录能命中新 Provider
- 精确模型验证：`model.alias=<model>@<provider_instance_name>` 能强制命中新 Provider

常用验证：

```bash
cargo test -p aicc
```

或在 `src` 下跑完整构建：

```bash
uv run buckyos-build.py
```

## 7. 常见问题

- 只注册 Provider，没写 `ModelRegistry`：`models.list` 能看到 Provider 但逻辑路由无候选。
- `logical_mounts` 写错：`model.alias=llm.chat` 等逻辑名无法命中。
- `api_types` 漏写：Provider 看起来有模型，但目标 method 被过滤。
- 把 `provider_type` 写成 `openai` / `claude`：这会破坏本地/云端/代理的安全策略判断。
- 精确模型名没带 provider instance：无法表达强制指定 Provider。
- 只改 AICC 不改 control_panel：后端可用但用户无法配置、诊断或 reload。
- 只维护 legacy alias：短期兼容可用，但不符合新版路由语义。
