# AICC 用例 Manifest、报告 Schema 与诊断

定义用例命名、manifest 字段、summary.json / summary.md schema、attempt、失败分类、诊断字段和报告示例。

本文档是拆分后的自包含验收任务文档。实现或评审本任务时，以本文档和 README 中列出的依赖文档为准。

## 1. 报告 Schema

`summary.json` 推荐结构：

```json
{
  "run_id": "20260509-acceptance-001",
  "started_at": "2026-05-09T00:00:00Z",
  "finished_at": "2026-05-09T00:10:00Z",
  "status": "failed",
  "summary": {
    "total": 120,
    "passed": 115,
    "failed": 2,
    "skipped": 2,
    "partial": 1
  },
  "layers": [
    {
      "layer": "L1",
      "status": "passed",
      "total": 60,
      "passed": 60,
      "failed": 0
    }
  ],
  "cases": [
    {
      "case_id": "routing_exact_model_no_fallback",
      "layer": "L1",
      "priority": "P0",
      "method": "llm.chat",
      "model": "gpt-5.2@openai-mock-1",
      "provider": "openai-mock-1",
      "status": "passed",
      "attempts_total": 1,
      "passed_after_attempt": 1,
      "error_code": null,
      "trace_id": "trace-aicc-001",
      "duration_ms": 20,
      "cost": {
        "amount": 0,
        "currency": "USD"
      },
      "artifacts": [],
      "skip_reason": null,
      "failure_reason": null,
      "attempts": [
        {
          "attempt": 1,
          "status": "passed",
          "failure_class": null,
          "error_code": null,
          "duration_ms": 20,
          "task_id": null,
          "trace_id": "trace-aicc-001"
        }
      ]
    }
  ]
}
```

报告状态：

| 状态 | 含义 |
|---|---|
| `passed` | 用例通过 |
| `failed` | 用例失败 |
| `skipped` | 前置条件缺失，例如未配置真实模型 key |
| `partial` | 真实模型协议链路成功，但模型内容或 Provider 状态导致部分断言不可稳定成立 |

退出码：

| 退出码 | 含义 |
|---:|---|
| 0 | 无失败 |
| 1 | 有失败 |
| 2 | 无失败但有 partial |

## 2. 用例命名规范

用例 ID 采用稳定、可检索、可映射到需求的格式：

```text
<layer>_<domain>_<feature>_<scenario>
```

字段约定：

| 字段 | 示例 |
|---|---|
| `layer` | `l1`、`l2`、`l3`、`l4` |
| `domain` | `routing`、`route_resolve`、`typed_inference`、`helper`、`metadata`、`overlay`、`scheduler`、`provider`、`protocol`、`resource`、`task`、`usage`、`security`、`settings`、`gateway`、`maintenance` |
| `feature` | `exact_model`、`fallback`、`min_line`、`auto_mount`、`variant_lowering`、`inherit`、`replace`、`stream_merge`、`reload_settings`、`named_object` |
| `scenario` | `success`、`no_fallback`、`rate_limit`、`policy_rejected`、`conflict` |

示例：

```text
l1_routing_exact_model_success
l1_routing_exact_model_no_fallback
l1_scheduler_weight_profile_cost_first
l1_provider_openai_stream_merge
l2_client_idempotency_conflict
l3_krpc_reload_settings_mock_openai
l3_resource_named_object_image_txt2img
l3_maintenance_metadata_openai_gpt_5_mini_logical_llm_chat
l3_maintenance_policy_openai_gpt_5_mini_cost_fallback
l4_gateway_openai_gpt_5_4_complex_workflow
l4_gateway_fal_video_upscale_workflow
l4_maintenance_release_openai_gpt_5_mini_related_cases
```

命名要求：

- case id 一旦进入报告，不应随意改名。
- case id 应能从名字看出层级、功能域和主要断言。
- 需求追踪矩阵中的用例族可用前缀表达，例如 `l1_routing_exact_model_*`。
- 维护更新类用例命名应能按更新内容筛选，建议包含更新类型、provider driver、provider instance 或厂商名、model id 或 model family、api type、逻辑目录和场景。manifest tags 应同步维护 `update:metadata`、`update:policy`、`update:routing`、`provider:<driver>`、`model:<id>`、`api_type:<method>`、`logical:<catalog>` 等字段；tags 尚未落地时，必须用 case id 关键词保证可检索。

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

## 4. 失败分类与诊断信息

报告中的失败原因应使用稳定分类，便于统计和自动处理。

| failure_class | 含义 |
|---|---|
| `preflight_failed` | 环境或依赖检查失败 |
| `config_failed` | TOML、settings、manifest 或 fixture 配置错误 |
| `service_unavailable` | AICC、task-manager、gateway 或 Mock Provider 不可访问 |
| `routing_failed` | 路由解析、候选、fallback、调度不符合预期 |
| `provider_protocol_failed` | Provider request/response 转换错误 |
| `provider_runtime_failed` | Provider 运行时返回错误、超时或状态异常 |
| `task_lifecycle_failed` | task 状态、event_ref、cancel、final result 不符合预期 |
| `resource_failed` | ResourceRef、artifact、FileObject meta 或读取失败 |
| `usage_failed` | usage 缺失、重复、查询不正确 |
| `security_failed` | 权限、隐私、脱敏失败 |
| `assertion_failed` | 用例断言失败 |
| `cleanup_failed` | 清理阶段失败 |

每个 failed case 至少记录：

- `case_id`
- `failure_class`
- `error_code`
- `message`
- `trace_id`
- `task_id`
- `provider`
- `model`
- `duration_ms`
- 脱敏后的 request 摘要
- 脱敏后的 response / error 摘要
- 相关日志片段位置

## 5. 验收报告示例

`summary.md` 建议面向人工阅读，保留失败定位信息和跳过原因。示例：

```markdown
# AICC Acceptance Report

- Run ID: 20260509-acceptance-001
- Mode: acceptance_all
- Gateway group: aicc-acceptance-20260509-001
- Started: 2026-05-09T10:00:00Z
- Finished: 2026-05-09T10:04:31Z
- Status: failed

## Summary

| Layer | Passed | Failed | Skipped | Not Applicable | Partial | Duration |
|---|---:|---:|---:|---:|---:|---:|
| L1 | 18 | 0 | 0 | 0 | 0 | 42s |
| L2 | 3 | 0 | 0 | 0 | 0 | 8s |
| L3 | 10 | 1 | 1 | 0 | 0 | 3m41s |
| L4 | 72 | 1 | 30 | 184 | 1 | 41m12s |

Total: 103 passed, 2 failed, 31 skipped, 184 not_applicable, 1 partial.

## L4 Matrix

| Provider | Api Types | Logical Paths | Models | Planned | Passed | Failed | Skipped | Not Applicable | Partial | Attempts |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| openai | 7 | 14 | 8 | 36 | 36 | 0 | 0 | 76 | 0 | 40 |
| claude | 2 | 7 | 3 | 12 | 12 | 0 | 0 | 30 | 0 | 12 |
| google-gemini | 6 | 12 | 5 | 20 | 19 | 1 | 0 | 40 | 0 | 25 |
| fal | 5 | 5 | 4 | 12 | 12 | 0 | 0 | 8 | 0 | 12 |
| openrouter | 1 | 6 | 5 | 0 | 0 | 0 | 30 | 0 | 0 | 0 |
| sn-ai-provider | 1 | 6 | 3 | 9 | 9 | 0 | 0 | 9 | 0 | 9 |

## L4 Matrix Detail

| Case | Api Type | Method | Logical Path | Provider | Exact Model | Logical Result | Exact Result | Attempts |
|---|---|---|---|---|---|---|---|---:|
| l4_gateway_llm_llm_chat_llm_plan_openai_gpt_5_5_pro | llm | llm.chat | llm.plan | openai | gpt-5.5-pro@openai | passed | passed | 1 |
| l4_gateway_vision_ocr_vision_ocr_image_ocr_google_florence_2 | vision.ocr | vision.ocr | image.ocr | google-gemini | florence-2@google | failed | not_run | 3 |
| l4_gateway_image_upscale_image_upscale_image_upscale_fal_esrgan | image.upscale | image.upscale | image.upscale | fal | esrgan@fal | passed | passed | 1 |

## Failed Cases

### l3_krpc_provider_5xx_failover

- Failure class: routing_failed
- Method: llm.chat
- Model: llm.plan
- Provider: openai-mock-1
- Error code: AICC_ROUTE_NO_CANDIDATE
- Trace ID: trace-aicc-20260509-0008
- Task ID: aicc-20260509-0008
- Duration: 3021ms
- Reason: expected failover to openai-mock-2, but no fallback candidate remained after hard filter.
- Artifacts: local_krpc/l3_krpc_provider_5xx_failover/

## Skipped Cases

| Case | Reason |
|---|---|
| l4_gateway_openai_* | allow_real_model_calls=false |
| l4_gateway_claude_* | missing api_key:claude |
| l4_gateway_openrouter_* | missing api_key:openrouter |

## Not Applicable Cases

| Dimension | Count | Reason |
|---|---:|---|
| vision.* × openrouter LLM models | 30 | model api_types do not contain vision api_type |
| image.upscale × openai text models | 8 | model not mounted to logical path and not admitted by min_line |

## Cost

| Provider | Real calls | Attempts | Estimated cost |
|---|---:|---:|---:|
| openai | 5 | 5 | USD 0.42 |
| claude | 3 | 3 | USD 0.31 |
| google-gemini | 7 | 7 | USD 0.28 |
| fal | 4 | 4 | USD 0.16 |
| openrouter | 0 | 0 | USD 0 |
| sn-ai-provider | 3 | 3 | USD 0 |

## Cleanup

- Temporary group: cleaned
- Cleanup warnings: 0

## Security Scan

- API keys: passed
- Session tokens: passed
- Raw prompt leakage: passed
- Raw file content leakage: passed
```

报告要求：

- `summary.md` 面向人工阅读。
- `summary.json` 面向 CI、脚本和后续分析。
- 失败用例必须提供 trace id、task id、failure class 和脱敏输入输出目录。
- skipped 不能只显示数量，必须显示原因。
- L4 报告必须显示真实模型调用次数、attempt 次数、`api_type × method × logical_path × Provider × model` 覆盖矩阵、not_applicable/skipped 原因和估算成本。
- L4 报告必须显示临时 group 是否已清理；如未清理，必须显示保留原因和手工清理命令。

