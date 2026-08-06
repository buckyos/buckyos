# AICC 维护更新验收用例

定义新增模型、新 Provider、新逻辑目录、metadata、routing、运营策略和回滚的维护更新验收闭环。

本文档是拆分后的自包含验收任务文档。实现或评审本任务时，以本文档和 README 中列出的依赖文档为准。

## 1. 发布验收标准

发布前建议满足以下硬指标：

1. P0 Mock 用例 100% 通过。
2. `cargo test -p aicc` 通过。
3. `cargo test -p buckyos-api --test aicc_client_test` 通过。
4. 本地 kRPC Mock 验收能完成 `reload_settings -> models.list -> route -> provider call -> task / usage / trace` 闭环。
5. gateway runner 能读取 TOML 配置并生成 `summary.md` 和 `summary.json`。
6. gateway runner 能通过 `buckyos-devkit` 启动临时 group，并从宿主机经 gateway 完成访问。
7. 已配置真实 key 的 Provider 必须覆盖其全部可用模型；`sn-ai-provider` 必须无普通 API key 覆盖；未配置 key 的 Provider 在普通开发验收中标记为 `skipped`，发布强覆盖验收中应 preflight 失败。
8. 报告、trace、task data、日志摘要中不得出现 API key、session token、原始 prompt 全文和原始文件内容。
9. 真实模型调用次数、attempt 次数和成本在报告中可见。
10. 所有 failed / partial 用例都有明确失败原因、错误码或 Provider 摘要。
11. runner 创建的临时 group 已清理，或报告中明确记录保留原因和清理命令。

### 1.1 新模型维护更新验收

当验收目标来自 `maintenance/aicc_maintenance_roles.md` 中的新模型、新 Provider、新逻辑目录挂载、metadata、运营策略或 routing 维护动作时，除满足常规发布标准外，还必须执行本节闭环。

维护更新类型：

| 类型 | 交付物 | 必验内容 |
|---|---|---|
| 已有 Provider 新增协议兼容模型 | 模型事实 metadata、运营策略、必要的 routing_config | `models.list` 出现新 exact model；`api_types`、`capabilities`、上下文长度、`logical_mounts` 正确；成本、健康度、权重和 fallback 策略生效 |
| 新增 OpenAI-compatible Provider instance | provider settings、`base_url`、授权、models 列表、metadata override | Provider 启用后 inventory 可见；exact model 可调用；逻辑目录可路由；缺 key / 错 key / `/models` 不兼容时错误可诊断 |
| 新增非兼容 Provider adapter 或新 API type | 版本包、adapter、schema、metadata 基线、默认路由策略 | 新 adapter 的协议转换、错误映射、streaming / task 语义、usage、fallback 和 helper / typed inference 链路通过相关用例 |
| 仅更新运营策略 | 策略配置、成本 / quota / health / 权重 / 熔断 / 灰度规则 | 不改变模型事实；route trace 显示策略命中；回滚策略后路由恢复；不需要回滚 metadata |
| 随版本内置缓存更新 | 版本包内 builtin metadata / 默认策略 | 新安装或无云端更新环境中仍能识别发布时已知模型，并生成可用默认路由 |
| 云端 metadata 更新 | 配置的 HTTPS/NDN 发布源；客户端内部 activation 位于 `$BUCKYOS_ROOT/data/srv/aicc/driver_metadata/remote_cache/v1/<source-key>/` | 完整验证后原子生效；损坏候选不破坏 LKGS；revision 回退和冲突被拒绝 |
| 人工运行时覆盖 | `$BUCKYOS_ROOT/etc/aicc/driver_metadata/local/<driver>.json` 或 `system-config/<driver>.json` | `reload_settings` 后生效；优先级高于 cloud activation；损坏配置被拒绝且不破坏可用基线 |

统一验收顺序：

1. 准备更新说明，列出 provider、model、api type、逻辑目录、模型事实变更、运营策略变更、routing 变更、是否需要 adapter 发版，以及影响的旧用例族。
2. 新增或更新命名可检索的相关用例，并在 manifest tags 中标明更新类型、provider、model、api type 和逻辑目录。
3. 在测试环境发布云端配置、运行时覆盖文件或版本包，触发 `reload_settings`。
4. 先执行本次新增用例和受影响旧用例，覆盖 inventory、metadata 解析、exact model、logical model、fallback、成本估算、禁用策略和错误返回。
5. 相关用例通过后执行 AICC 全量用例，确认旧 Provider、旧模型和旧路由策略未回归。
6. 发布环境上线后重复相关用例，再执行发布环境全量用例；发布环境的授权、网络、Provider 实际状态和报告摘要必须可诊断。
7. 如本次支持回滚，至少执行一次目标回滚用例：模型事实错误时回滚 metadata / override；路由错误时优先回滚运营策略或 routing_config；回滚后重新 `reload_settings`，确认 `models.list`、route trace 和关键调用恢复预期。

角色边界：

- BuckyOS 项目方更新公共协议、默认模型事实基线、默认运营策略基线、默认逻辑目录和随版本缓存时，必须同时提交或更新对应 L1/L3/L4 用例。
- 商业服务商跟随 BuckyOS 更新或维护自有 Provider 网关、模型事实包、运营策略包和产品默认 routing_config 时，必须保留服务商维度的用例 tags，报告中能按服务商 Provider / model 聚合。
- 产品用户通过 system_config、local metadata override 或 `session_overlay` 做临时接入时，验收只要求配置生效、可回滚和安全边界正确；不要求修改公共基线。
- 模型服务商主动提供 BuckyOS metadata / inventory / cost / quota / health 信息目前按畅想处理；如接入试点，应作为服务商或第三方 Provider 包验收，不作为 P0 默认要求。

## 2. 文档联动要求

后续实现测试或修改 AICC 协议时，需要同步检查：

- `doc/aicc/aicc_api设计.md`
- `doc/aicc/aicc_router.md`
- `doc/aicc/aicc 逻辑模型目录.md`
- `doc/aicc/maintenance/krpc_aicc_calling_guide.md`
- `doc/aicc/maintenance/update_aicc_settings_via_system_config.md`
- `doc/aicc/aicc_provider_plan.md`
- `doc/aicc/aicc_usage_log_db_requirements.md`
- `doc/aicc/maintenance/aicc_maintenance_roles.md`
- `src/kernel/buckyos-api/src/aicc_client.rs`
- `src/frame/aicc/src`
- `test/aicc_test`

触发文档联动的变更包括：

1. 新增或改名 method。
2. 修改 request / response schema。
3. 修改 `ResourceRef` JSON 表达。
4. 修改 Provider settings 字段。
5. 修改 exact model 命名规则。
6. 修改 fallback、session config、policy 字段。
7. 修改 usage log schema。
8. 修改 task data / event 中 AICC 字段。
9. 修改 metadata、运营策略、`remote_cache`、provider settings、routing_config 或回滚流程。

## 3. 用例 Manifest 约定

为便于统一 runner 执行和生成报告，建议把 L3/L4 用例声明为 manifest。Rust L1/L2 可以不强制使用 manifest，但报告中的 case metadata 应与 manifest 字段保持一致。

推荐文件：

```text
test/aicc_test/cases/
  local_mock_cases.toml
  gateway_cases.toml
```

Manifest 样例：

```toml
[[cases]]
case_id = "l3_krpc_llm_chat_json_schema_success"
layer = "L3"
priority = "P0"
method = "llm.chat"
model_alias = "llm.plan"
provider = "openai-mock-1"
scenario = "success"
timeout_ms = 30000
requires = ["mock_provider", "aicc_service", "task_manager"]
fixtures = []
expect_status = "succeeded"
expect_artifacts = false
expect_usage = true
expect_trace = true

[cases.input]
template = "llm_chat_json_schema.json"

[cases.assertions]
json_schema = "assertions/llm_chat_summary.schema.json"
no_sensitive_log = true

[[cases]]
case_id = "l4_gateway_${api_type_slug}_${method_slug}_${logical_slug}_${provider}_${model_slug}"
layer = "L4"
priority = "P2"
api_type = "${api_type}"
method = "${method}"
logical_path = "${logical_path}"
provider = "openai"
model = "${exact_model}"
matrix_source = "models.list"
timeout_ms = 300000
requires = ["gateway", "real_model", "api_key:openai"]
max_attempts = 3
expect_status = "partial_or_passed"
expect_usage = true
expect_trace = true

[[cases]]
case_id = "l3_maintenance_metadata_openai_gpt_5_mini_logical_llm_chat"
layer = "L3"
priority = "P1"
method = "maintenance.update"
provider = "openai-mock-1"
model = "gpt-5-mini@openai-mock-1"
update_type = "metadata"
api_types = ["llm.chat"]
logical_catalogs = ["llm.chat"]
tags = ["update:metadata", "provider:openai", "model:gpt-5-mini", "api_type:llm.chat", "logical:llm.chat"]
requires = ["mock_provider", "aicc_service", "settings_reload"]
expect_status = "succeeded"
expect_usage = false
expect_trace = true
```

字段说明：

| 字段 | 说明 |
|---|---|
| `case_id` | 稳定用例 ID，进入报告后不随意变更 |
| `layer` | `L1`、`L2`、`L3`、`L4` |
| `priority` | `P0`、`P1`、`P2` |
| `api_type` | canonical `ApiType` 序列化值，例如 `llm`、`vision.ocr`、`image.txt2img` |
| `method` | AICC method 或 `workflow` |
| `logical_path` | 标准逻辑目录路径，例如 `llm.plan`、`image.ocr`；不得用 `vision.ocr` 代替 `image.ocr` |
| `model_alias` | 请求模型名，可为逻辑模型或精确模型 |
| `provider` | 期望命中的 Provider；路由类用例可为空 |
| `provider_driver` | Provider driver 名，例如 `openai`、`google-gemini`、`claude` |
| `scenario` | Mock 行为场景 |
| `update_type` | 维护更新类型，例如 `metadata`、`policy`、`routing`、`provider_settings`、`adapter_release`、`rollback` |
| `api_types` | 本用例覆盖的 AICC method / API type 列表；L4 矩阵用例必须同时填写单值 `api_type` |
| `logical_catalogs` | 本用例覆盖的逻辑目录列表 |
| `tags` | 可检索标签；维护更新类用例至少包含 `update:*`、`provider:*`、`model:*`、`api_type:*` 或 `logical:*` 中适用项 |
| `requires` | 前置能力；缺失时用例 `skipped` |
| `fixtures` | 所需 fixture 列表 |
| `expect_status` | `succeeded`、`running`、`failed`、`partial_or_passed` |
| `expect_usage` | 是否必须存在 usage |
| `expect_trace` | 是否必须存在 route trace |
| `max_attempts` | L4 单 case 最大 attempt 数；真实模型默认 3 |
| `matrix_source` | L4 动态矩阵来源，推荐 `models.list` |
| `model_slug` | 由精确模型名归一化得到的稳定用例 ID 片段 |
| `logical_slug` | 由逻辑目录路径归一化得到的稳定用例 ID 片段 |
| `logical_attempt` | 报告字段：逻辑模型段 attempt 摘要，包含 requested logical path、route trace、selected exact model |
| `exact_attempt` | 报告字段：物理模型段 attempt 摘要，包含 exact model、provider、usage、trace 和 no-fallback 断言 |

Runner 要求：

- manifest 解析失败应直接终止，不能静默跳过。
- `requires` 不满足时标记 `skipped`，并记录 `skip_reason`。
- 同一 manifest 内 `case_id` 必须唯一。
- 报告中的 case 顺序应与 manifest 顺序一致，便于人工阅读。
- L4 动态矩阵用例可以由模板 case 展开；展开后的 `case_id` 必须唯一，并保留 `api_type`、`method`、`logical_path`、`provider`、`model`、`matrix_source`。
- L4 attempt 明细必须挂在同一个 case 下，不能展开成多个独立 case 影响通过率统计。
- 维护更新类用例必须支持按 `update_type`、`provider_driver`、`provider`、`model`、`api_types`、`logical_catalogs` 和 `tags` 筛选；报告中应能单独汇总本次更新相关用例与全量回归用例。

