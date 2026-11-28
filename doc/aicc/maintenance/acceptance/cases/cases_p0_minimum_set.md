# AICC 首批 P0 最小用例集

定义 M0/M1 阶段必须先落地的稳定 P0 case id、层级和目标。

本文档是拆分后的自包含验收任务文档。实现或评审本任务时，以本文档和 README 中列出的依赖文档为准。

## 1. 首批 P0 最小用例集

为避免第一轮实现范围过大，M0/M1 阶段先落地以下最小 P0 用例集。该集合不追求覆盖全部 method，而是优先打通协议、路由、Mock、任务、资源、usage、trace 和异常主链路。

### 1.1 M0 最小集

| case id | 层级 | 目标 |
|---|---|---|
| `l1_routing_exact_model_success` | L1 | 精确模型名解析成功 |
| `l1_routing_exact_model_no_fallback` | L1 | 精确模型不可用时直接失败，从不换模型 |
| `l1_routing_logical_model_candidates` | L1 | 逻辑模型展开候选列表 |
| `l1_routing_no_policy_no_switch` | L1 | 用途没有不可用时策略：同池两个 Provider，选中者连续失败 10 次，另一个收到 0 次请求；404 之后再请求仍选同一模型 |
| `l1_routing_fallback_once` | L1 | 用途配置 `same_model` 时最多切换一次；备选也失败时错误同时带两边原因 |
| `l1_scheduler_weight_priority` | L1 | 目录 item weight 优先级生效 |
| `l1_scheduler_profile_balanced_default` | L1 | 未指定 profile 时按 `balanced` 选择 |
| `l1_usecase_layer_override_route` | L1 | 用户层为用途选的精确模型覆盖管理员层和默认路由；管理员锁定的格子不被覆盖 |
| `l1_request_policy_tighten_only` | L1 | 请求策略只能收紧：不替换管理员策略，允许列表与管理员的取交集，不能改锁定的 profile |
| `l1_routing_fallback_llm_coverage` | L1 | 兜底 LLM 关闭时没有模型的 LLM 用途返回 `no_candidate_model`（`details.usecase`）；打开后由 `llm.fallback` 提供，`route_trace.fallback.kind = coverage` |
| `l1_security_local_only_rejects_cloud` | L1 | `local_only` 硬过滤云端 Provider |
| `l1_provider_openai_chat_success` | L1 | OpenAI-like `chat.completions.create` 协议转换成功 |
| `l1_provider_openai_stream_merge` | L1 | Provider streaming chunks 聚合为最终 summary |
| `l1_resource_ref_json_tags` | L1 | `url`、`base64`、`named_object` JSON tag 正确 |
| `l1_task_immediate_success` | L1 | 同步成功任务写入 result |
| `l1_task_async_success` | L1 | 异步任务 running 到 succeeded 闭环 |
| `l1_usage_success_write_once` | L1 | 成功调用写入 exactly one usage event |
| `l1_usage_missing_usage_rejected` | L1 | 成功响应缺 usage 被判为协议错误 |
| `l2_client_llm_chat_success` | L2 | `AiccClient::chat_completions_create` 调用成功 |
| `l2_client_idempotency_conflict` | L2 | 同 key 不同 body 返回 idempotency conflict |
| `l2_client_cancel_unknown_task` | L2 | 取消不存在任务返回可判断错误 |

### 1.2 M1 最小集

| case id | 层级 | 目标 |
|---|---|---|
| `l3_settings_reload_mock_openai` | L3 | 写入 Mock settings 后 reload 生效 |
| `l3_models_list_mock_inventory` | L3 | `models.list` 可看到 Mock Provider inventory |
| `l3_krpc_chat_completions_create_text_success` | L3 | kRPC `chat.completions.create` 纯文本成功 |
| `l3_krpc_chat_completions_create_json_schema_success` | L3 | JSON schema 输出可解析 |
| `l3_krpc_resource_base64_image` | L3 | base64 图片资源输入成功 |
| `l3_krpc_resource_named_object_artifact` | L3 | named_object artifact 输出可读取 |
| `l3_krpc_stream_progress_and_final` | L3 | streaming 中间态写 task data，最终 summary 正确 |
| `l3_krpc_async_task_success` | L3 | 异步任务状态闭环 |
| `l3_krpc_provider_5xx_no_policy_real_error` | L3 | 用途没有策略时 Provider 5xx 返回真实错误，不换模型 |
| `l3_krpc_provider_5xx_same_model_switch` | L3 | 用途配置 `same_model` 时 Provider 5xx 后切到另一个 Provider 的同一模型，trace、usage、事件标出原模型 |
| `l3_krpc_marked_faulty` | L3 | Provider 标记为故障：有策略的用途直接用备选，没有策略的立即返回 `no_provider_available`（`details.reason = marked_faulty`），用途表不变 |
| `l3_krpc_provider_timeout_failed` | L3 | Provider timeout 返回明确错误 |
| `l3_krpc_usage_query_last_1d` | L3 | usage 可按 last_1d 查询 |
| `l3_krpc_security_no_secret_in_report` | L3 | 报告和 trace 脱敏扫描通过 |

2026-11-27 删除的最小集用例：`l1_routing_parent_fallback_success`（不再有 parent fallback）、`l1_routing_fallback_loop_rejected`（注册表没有 fallback 链，没有环可测）、`l1_scheduler_profile_cost_first`（`cost_first` 已删除）、`l1_request_overlay_override_route` / `l1_request_overlay_stateless`（请求不再携带 `session_overlay`）、`l3_krpc_provider_5xx_failover`（没有策略时不换模型，换模型只来自用途的不可用时策略）。规则见 [aicc_router.md §9](../../../aicc_router.md#9-fallback-与失败处理)、[§11](../../../aicc_router.md#11-用途表与配置层次)。

首批 P0 最小集通过后，再扩展到完整 P0/P1/P2 用例矩阵。
