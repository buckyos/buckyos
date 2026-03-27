# AICC 新增 Provider 开发指南（参考 commit `8d169271`）

本文基于 `8d1692712ef4e80615bd49b6d3b9e46422999072`（新增 Claude Provider）和 AICC 当前实现，总结一套可复用的 Provider 接入流程，目标是让开发者可以按同一套路新增任意模型厂商 Provider。

## 1. 先理解 AICC 的接入边界

AICC 的 Provider 接口定义在 `src/frame/aicc/src/aicc.rs`，核心约束是：

- 每个 Provider 必须实现 `Provider` trait：
  - `instance()`: 返回 Provider 实例元信息（`instance_id`、`provider_type`、`capabilities`、`features` 等）
  - `estimate_cost()`: 给路由器打分时用的成本/延迟估计
  - `start()`: 真正调用上游模型 API
  - `cancel()`: 可取消任务的 Provider 需实现；不支持可返回 `Ok(())`
- 路由并不直接认识“某厂商模型名”，而是走 `ModelCatalog` 的 alias 映射：
  - `alias + capability + provider_type -> provider_model`
- Provider 的可见性来自 `Registry`，路由只会在 Registry 里挑实例。

结论：新增 Provider 的本质，是“注册 Provider 实例 + 注册 alias 映射 + 实现 start/错误分类”。

## 2. 对照 Claude 实现的最小改动面

参考 `8d169271`，新增 `claude` 时改了这些关键点：

- 新增 Provider 模块：`src/frame/aicc/src/claude.rs`
- 复用/新增协议转换层：`src/frame/aicc/src/claude_protocol.rs`
- 在 crate 对外导出模块：`src/frame/aicc/src/lib.rs`
- 在服务入口注册 Provider：`src/frame/aicc/src/main.rs` 的 `apply_provider_settings()`
- 增加 adapter 协议测试：`src/frame/aicc/tests/adapter_protocol_tests.rs`
- 控制面板后端补 Provider 卡片/保存逻辑：`src/frame/control_panel/src/main.rs`
- 控制面板 web mock/API 对齐：`src/frame/control_panel/web/src/api/index.ts`
- （可选但建议）补系统默认配置生成：`src/kernel/scheduler/src/system_config_builder.rs`
- （同次提交但非 Provider 接入必需）`src/rootfs/bin/buckyos_jarvis/behaviors/*.yaml`

可直接把这套“落点清单”作为你新增 Provider 的 checklist。

## 3. 新增一个 Provider 的标准步骤

## 步骤 1：新增 `<provider>.rs`，定义实例配置与 Provider 结构

建议直接参照 `claude.rs` / `minimax.rs` 的结构：

- `XXXInstanceConfig`：实例级配置（`instance_id`、`provider_type`、`base_url`、`timeout_ms`、`models`、`default_model`、`features`、`alias_map`）
- `XXXProvider`：
  - `instance: ProviderInstance`
  - `client: reqwest::Client`
  - `api_token`、`base_url`
- `new(cfg, api_token)` 中组装 `ProviderInstance`，声明支持的 capability（例如 `Capability::LlmRouter`）。

## 步骤 2：实现 Provider 协议转换层（建议独立文件）

若上游 API 与 AICC `CompleteRequest` 不同，建议新增 `<provider>_protocol.rs`，职责清晰：

- 将 AICC 请求转换为上游请求 JSON
- 做参数白名单/兼容转换（例如 `tool_choice`、`tools`、`response_format`）
- 在“请求不合法”时直接返回 `ProviderError::fatal(...)`

Claude 就走了这条路线：`claude.rs` 调用 `claude_protocol::convert_complete_request(...)`。

## 步骤 3：实现 `start()` 与错误分类

这是稳定性关键：

- `start()` 按 capability 分发（例如 `LlmRouter` -> `start_llm(...)`）
- HTTP 429、5xx、网络错误统一标记 `retryable`
- 参数错误、协议错误、解析错误通常标记 `fatal`
- 返回 `ProviderStartResult::Immediate(AiResponseSummary { ... })` 时建议把 `provider_io` 放到 `extra` 里，便于排障

可直接参考 `claude.rs` 的 `classify_api_error()` 和 `start_llm()`。

## 步骤 4：实现 settings 解析与实例构建

每个 Provider 都应有自己的 settings 解析函数：

- `parse_<provider>_settings(settings: &Value) -> Result<Option<...>>`
  - 不存在或 `enabled=false` 返回 `Ok(None)`
  - `api_token` 缺失时报错
- `build_<provider>_instances(...)`
  - 填默认模型、默认特性
  - 清洗模型列表（去空、去重）

建议支持 `api_key`/`api_token` 双别名，降低配置迁移成本（Claude 已实现）。

## 步骤 5：注册 Provider 到 Registry 和 ModelCatalog

新增 `register_<provider>_providers(center, settings)`：

- 创建 Provider 实例并 `center.registry().add_provider(provider)`
- 注册默认 alias（至少覆盖）：
  - `llm.<model>`
  - `llm.default`
  - `llm.chat.default`
  - `llm.plan.default`
  - `llm.code.default`
- 注册自定义 `alias_map`（全局 + 实例级）

如果 Provider 支持多 capability（如 `LlmRouter + Text2Image`），alias 注册时要按 alias 前缀路由到正确 capability（见 `openai.rs` / `gimini.rs`）。

## 步骤 6：把模块接进启动链路

两处必须改：

- `src/frame/aicc/src/lib.rs`：`pub mod <provider>;`
- `src/frame/aicc/src/main.rs`：
  - `mod <provider>;`
  - `use crate::<provider>::register_<provider>_providers;`
  - 在 `apply_provider_settings()` 中调用注册函数并累加 `registered_total`

注意：当前入口是“全量清空后重建”注册表，新增 Provider 要遵守这个初始化模式。

## 步骤 7：补充配置下发（建议）

如果你希望开箱即用，需在系统配置构建里补默认配置：

- `src/kernel/scheduler/src/system_config_builder.rs`
  - 在 `build_aicc_settings()` 插入 `<provider>` 配置块
  - 给出 `alias_map` + `instances` 的默认样例

这样用户在安装/启动后就能直接得到结构正确的 settings。

## 步骤 8：测试分层（必须）

至少补两类测试：

- Provider 本地单元测试（建议放 `<provider>.rs` 内 `#[cfg(test)]`）：
  - 默认实例构建
  - alias 注册与解析
- adapter 协议测试（`src/frame/aicc/tests/adapter_protocol_tests.rs`）：
  - 200 成功
  - 429 retryable
  - 4xx fatal
  - 网络/超时 retryable

Claude 在 `8d169271` 就是按这套补齐。

## 步骤 9：补齐控制面板/UI 配置接入（建议）

如果你希望“新增 Provider 后用户可在控制面板里查看、编辑、诊断”，需要同时接入 `control_panel`：

- 控制面板后端 Provider 卡片：
  - 文件：`src/frame/control_panel/src/main.rs`
  - 在 `ai_provider_cards(...)` 中补 `<provider>` 对应的 card 生成逻辑（可参考 `ai_claude_provider_card` / `ai_minimax_provider_card`）
  - 统一处理 `status`、`credentialConfigured`、`maskedApiKey`、`defaultModel`、`endpoint`
- 控制面板保存逻辑：
  - 文件：`src/frame/control_panel/src/main.rs`
  - 在 `handle_ai_provider_set(...)` 中为你的 `provider_id` 增加写回分支，把用户修改落到 `services/aicc/settings`
  - 建议同步维护 `provider_type`、`instances[0].base_url`、`instances[0].default_model`、`api_token/api_key` 兼容字段
- 控制面板诊断/重载链路：
  - 复用已有 `ai.provider.test` 与 `ai.reload` RPC（无需重复造接口）
  - 确认新增 Provider 在 reload 后能被 `ai.provider.list` 正确反映
- Web UI 页面与路由：
  - 文件：`src/frame/control_panel/web/src/routes/router.tsx`、`src/frame/control_panel/web/src/ui/pages/*.tsx`
  - 若已有 AI Provider 管理页，补展示字段与编辑表单；若无入口，需新增页面并挂路由
  - API 层使用 `src/frame/control_panel/web/src/api/index.ts` 的 `fetchAiProviders` / `saveAiProvider` / `runAiProviderDiagnostic` / `reloadAiProviderSettings`
- 文档同步：
  - 同步更新 `doc/aicc/local_provider.md` 或 Provider 专属文档，明确“系统配置入口”和“控制面板入口”两条路径

最小验收建议：

1. 控制面板可看到新 Provider 卡片（状态、模型、endpoint 正确）
2. 在 UI 修改默认模型/endpoint/API Key 后，`services/aicc/settings` 实际更新
3. 点击 reload 后，AICC 路由能命中新 Provider（非仅 UI 假状态）
4. 诊断接口能给出可读结果（成功或可定位错误码）

## 4. 推荐的 settings 模板

下面是一个最小可用模板（以 `myprovider` 为例）：

```json
{
  "myprovider": {
    "enabled": true,
    "api_token": "YOUR_TOKEN",
    "alias_map": {
      "llm.default": "model-a",
      "myprovider-fast": "model-b"
    },
    "instances": [
      {
        "instance_id": "myprovider-default",
        "provider_type": "myprovider",
        "base_url": "https://api.example.com/v1",
        "timeout_ms": 60000,
        "models": ["model-a", "model-b"],
        "default_model": "model-a",
        "features": ["plan", "json_output", "tool_calling"],
        "alias_map": {
          "llm.plan.default": "model-a"
        }
      }
    ]
  }
}
```

更新后调用 `reload_settings` 使其生效。

## 5. 常见坑（按出现频率）

- 只注册了 Provider，没注册 alias：路由会报 `model_alias_not_mapped`
- alias 绑定到了错误 capability：看起来“有映射”，实际仍不可路由
- 把 4xx 都标 retryable：会导致无意义重试放大故障
- `provider_type` 不一致（settings 和代码中不一致）：`ModelCatalog.resolve()` 命不中
- 忘记在 `main.rs` 接入 `register_*`：Provider 文件存在但永远不会启用
- 只改了 AICC 没改 control_panel：后端可用但 UI 不可配置/不可观测

## 6. 建议的开发顺序（最快闭环）

1. 复制 `claude.rs` 为模板改名，先跑通 `LlmRouter`
2. 接入 `lib.rs` + `main.rs` 注册链路
3. 补 settings 解析与 alias 注册
4. 先写 adapter 4 条协议测试（200/429/400/网络错误）
5. 最后补细节能力（tool calling、json_output、多 capability）

按以上顺序，通常可以最快把“可用 Provider”上线，再迭代高级能力。
