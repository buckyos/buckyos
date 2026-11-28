# AICC L3 本地 P0 核心用例

定义本地 kRPC P0 闭环，包括 settings reload、Mock Provider、models.list、chat.completions.create、resource、task、usage、trace 和错误分类。

本文档是拆分后的自包含验收任务文档。实现或评审本任务时，以本文档和 README 中列出的依赖文档为准。

## 1. 异常路径

必测异常：

| 类别 | 场景 |
|---|---|
| 路由 | 非法模型名、模型不存在、无候选（用途没有模型时 `details.usecase` 给出用途名）、策略拒绝、logical tree loop、精确模型不可用 |
| 不可用时 | 用途没有策略时任何 Provider 失败都返回真实错误，不改写成无候选；备选也失败时错误同时带两边原因；Provider 被标记为故障且用途没有策略时返回 `no_provider_available` |
| 策略 | `local_only` 无本地候选、预算超限、feature unsupported、context too long、provider 被禁用 |
| Provider | 401/403、429、5xx、timeout、quota exhausted、malformed response、missing usage、unsupported media type |
| 任务 | cancel unknown task、cancel forbidden、provider 不支持取消、异步任务最终失败 |
| 幂等 | 重复 key 命中 running/succeeded/failed/cancelled；相同 key 不同 body 返回 conflict |
| 配置 | settings schema 非法、凭据缺失、reload 后 provider 数量为 0、inventory 为空 |
| 管理 | provider validate/add/delete/refresh 无权限、重名、损坏 metadata、删除被引用 Provider、`models.list` / `provider.list` 敏感字段泄露 |
| 命名 | `route.resolve.api_type` 与正式 `ApiType` 枚举不一致、历史别名行为不稳定、inventory 中出现非正式 api_type |
| 安全 | 跨 tenant 查询/取消拒绝，trace/log 不包含 token、prompt 原文、原始文件内容 |

错误验收要求：

- 错误码可机器判断。
- 用户可见 message 可理解。
- route trace 记录候选过滤、来源层，以及兜底 LLM 或不可用时切换的原因（`fallback`）。
- Provider 原始错误只保留脱敏摘要。
- 早期 kRPC error 不创建 AICC task；已创建 task 的失败写入 task data / event。

## 2. 配置与重载验收

必须覆盖完整闭环：

```text
system_config 写入 settings
  -> service.reload_settings
  -> models.list
  -> route
  -> provider call
  -> usage / trace
```

用例：

1. 写入 Mock OpenAI Provider Instance，`base_url` 指向本地 Mock，reload 后 `models.list` 出现 `openai-mock-1`。
2. 禁用 Provider，确认向库存刷新定时任务循环发送 `Stop` 并等待优雅退出；reload 后候选消失，调用返回无候选或策略拒绝，且没有新探测或迟到的 inventory/health 写入。
3. 修改 Model Driver 的结构化能力，reload 后 `ModelRequirement` 硬过滤结果变化。
4. 修改 `provider_type` 为 `local_inference` / `cloud_api` / `proxy_unknown`，验证 `local_only` 过滤。
5. 全量覆盖 settings 和局部更新 settings 都能生效。
6. settings 非法时 reload 失败，不破坏上一版可用配置。
7. `provider.validate` 只做校验和脱敏诊断，不写入 system_config。
8. `provider.add` 写入后 reload，`models.list` / `provider.list` / `provider.health` 可见，且路由可命中新增 Provider。
9. `provider.refresh_models` 在 model 列表变化或 target/applied seq 不同时更新 inventory；列表未变且 seq 相同时只探测。seq 不一致时必须收敛所有落后 Provider，成功后 `models.list` 和 applied seq 同步变化。
10. `provider.delete`、配置替换实例和 AICC 服务停止都向对应库存刷新定时任务循环发送幂等 `Stop` 并等待退出，不遗留孤儿定时器；删除后 reload，候选和 provider list 中目标消失，若仍被 locked policy 引用则返回明确错误或诊断。

## 3. Usage Log 验收

用例：

1. 成功同步调用写入 exactly one usage event。
2. 成功异步最终完成写入 exactly one usage event。
3. 相同 `tenant_id + method + idempotency_key` 重试不重复写 usage event。
4. Provider 成功但缺 usage，调用应失败为 provider protocol error，不写成功 usage event。
5. TaskMgr completed task 删除后 usage event 仍可查询。
6. `last_1d` 按 provider model 汇总。
7. `last_7d` 按 provider model 汇总。
8. 自定义时间范围按 request model + provider model 汇总。
9. raw events 支持 `limit` / `cursor`。
10. finance snapshot 存在时写入，缺失时不影响成功。

## 4. 安全与脱敏验收

增加专门用例扫描报告、trace、task data、日志摘要，确认不出现：

- API key。
- session token。
- 原始 prompt 全文。
- 原始文件内容。
- Provider 原始敏感响应。

隐私策略用例：

1. `local_only=true` 时云端 Provider 被硬过滤。
2. `provider_type=proxy_unknown` 不被视为本地。
3. Provider inventory 自声明 `attributes.local=true` 但 system_config 中 `provider_type=cloud_api` 时，不能通过本地过滤。
4. 用户级策略不能覆盖组织级 locked policy。
5. 跨用户、跨租户查询和取消任务被拒绝。

## 5. 分层用例清单

本节把前文功能域拆成更接近实现任务的用例清单。用例 ID 可在实现时继续细化，但应保持前缀稳定。

### 5.1 L1 白盒单测

| 用例族 | 优先级 | 覆盖点 |
|---|---|---|
| `l1_routing_exact_model_*` | P0 | 精确模型解析、Provider instance 校验、API type 校验、从不换模型 |
| `l1_routing_logical_tree_*` | P0 | 逻辑目录展开、items target、目录软链接、候选去重 |
| `l1_routing_fallback_*` | P0 | 用途的不可用时策略 `none`（默认）/ `same_model` / `not_pricier` / `next_model`：触发与不触发类别、每次请求最多切换一次、备选沿用原请求与用途约束、请求 `no_fallback` 关闭；兜底 LLM 只为静态无候选的 LLM 用途提供；标记为故障 |
| `l1_routing_loop_*` | P0 | logical tree loop（目录软链接环）；注册表没有 fallback 链，不再有 fallback loop 和深度限制 |
| `l1_scheduler_weight_*` | P0 | item weight、exact model weight、weight 0 硬过滤、同权重 profile 评分 |
| `l1_scheduler_profile_*` | P0 | `latency_first`、`quality_first`、`balanced`（默认）、`local_first`、`strict_local`；`cost_first` 已删除 |
| `l1_usecase_layers_*` | P0 | 用途表各层合并（默认 < 管理员 < 用户 < App 组 < 会话组）、管理员锁定、层选的模型缺失 / 不可见 / 不满足要求时跟随下一层、请求策略只能收紧、互不污染（取代已删除的 `l1_request_overlay_*`：请求不再携带 `session_overlay`） |
| `l1_provider_protocol_openai_*` | P0 | Responses 与 Chat Completions 分 Adapter contract、无隐式 fallback |
| `l1_provider_protocol_claude_*` | P0 | Messages 与兼容 Completions 分 Adapter contract |
| `l1_provider_protocol_gemini_*` | P0 | Interactions 与 `generateContent` 分 Adapter contract、无隐式 fallback |
| `l1_provider_protocol_fal_*` | P1 | fal submit/poll、artifact URL、operation timeout |
| `l1_resource_ref_*` | P0 | `url`、`base64`、`named_object`、FileObject meta 推导 |
| `l1_task_lifecycle_*` | P0 | immediate、async running、final succeeded、failed、cancel |
| `l1_usage_log_*` | P0 | 成功写 usage、幂等去重、缺 usage 报错、查询聚合 |
| `l1_method_api_type_canonical_*` | P0 | `api_type=llm` 与 `method=chat.completions.create` 的边界、非正式 api_type 拒绝 |
| `l1_control_method_*` | P0 | cancel、reload、models list、usage/quota/provider 查询的 schema 和权限边界 |
| `l1_security_*` | P0 | `local_only`、`proxy_unknown`、locked policy、trace 脱敏 |
| `l1_concurrency_*` | P1 | session patch 并发、幂等并发、异步任务并发完成 |

### 5.2 L2 AiccClient 黑盒测试

| 用例族 | 优先级 | 覆盖点 |
|---|---|---|
| `l2_client_llm_chat_success` | P0 | `AiccClient::chat_completions_create` 构造 typed 请求并解析成功响应 |
| `l2_client_exact_model_no_fallback` | P0 | 精确模型不可用时透传可判断错误 |
| `l2_client_idempotency_*` | P0 | running / succeeded / failed / conflict 语义 |
| `l2_client_async_task_*` | P0 | running response、event_ref、最终 task 查询 |
| `l2_client_cancel_*` | P0 | cancel 成功、unknown task、forbidden |
| `l2_client_control_method_*` | P0 | reload、models list、usage/quota/provider 查询响应解析和错误映射 |
| `l2_client_resource_ref_*` | P1 | client 侧 `ResourceRef` JSON tag 和反序列化 |
| `l2_client_error_mapping_*` | P0 | kRPC error 与 AICC task failed error 的边界 |

### 5.3 L3 本地 kRPC 黑盒测试

| 用例族 | 优先级 | 覆盖点 |
|---|---|---|
| `l3_settings_reload_mock_*` | P0 | system_config 写入 Mock settings、reload、models.list |
| `l3_provider_admin_*` | P0 | provider.validate/add/delete/refresh_models 的 system_config 写入、reload、回滚，以及停止/禁用/删除/替换时库存定时循环的 `Stop` 与优雅退出语义 |
| `l3_models_list_*` | P0 | `models.list` inventory、完整身份链、逻辑目录、operations、health 脱敏诊断 |
| `l3_quota_query_*` | P1 | `quota.query` 按 tenant、capability、method 返回预算状态和拒绝路径 |
| `l3_krpc_chat_completions_create_*` | P0 | 纯文本、多模态 content part、tool call、JSON schema |
| `l3_krpc_resource_*` | P0 | `url`、`base64`、`named_object` 输入和 artifact 输出 |
| `l3_krpc_stream_*` | P0 | Mock streaming chunks、task data progress、final summary |
| `l3_krpc_async_*` | P0 | image/audio/video 类异步 task 状态闭环 |
| `l3_krpc_usage_*` | P0 | usage event 写入和查询 |
| `l3_krpc_unavailable_*` | P0 | 用途没有策略时 timeout / 5xx / quota exhausted 返回真实错误、不换模型；配置 `same_model` / `not_pricier` / `next_model` 后切换一次，`route_trace.fallback`、usage `fallback_from`、事件 `unavailable_fallback` 可见；标记为故障（取代已删除的 `l3_krpc_failover_*`） |
| `l3_krpc_security_*` | P0 | local_only、跨用户访问拒绝、脱敏扫描 |

### 5.4 L4 Gateway 真实模型验收

| 用例族 | 优先级 | 覆盖点 |
|---|---|---|
| `l4_gateway_openai_<model>_complex_workflow` | P2 | OpenAI 每个支持模型的文本、JSON schema、tool call、usage、trace |
| `l4_gateway_claude_<model>_complex_workflow` | P2 | Claude 每个支持模型的多模态或 vision、tool use、usage、trace |
| `l4_gateway_gemini_<model>_complex_workflow` | P2 | Google Gemini 每个支持模型的多模态、safety / function call / operation 语义 |
| `l4_gateway_openrouter_<model>_complex_workflow` | P2 | OpenRouter 每个支持模型的 OpenAI-compatible 协议兼容、usage、trace |
| `l4_gateway_fal_<model>_media_workflow` | P2 | fal 每个支持模型的 image/video/audio 工具型异步任务和 artifact |
| `l4_gateway_sn_ai_provider_<model>_complex_workflow` | P2 | SN AI Provider 每个支持模型的 API Key/动态登录双链路、token 刷新、usage、trace、provider 归因 |
| `l4_gateway_models_list` | P2 | 真实环境 inventory、逻辑目录和 Provider health 可诊断 |

L4 用例 ID 中的 `<model>` 必须使用稳定可读的 slug，由精确模型名归一化得到；报告中必须保留原始精确模型名。

## 6. 首批 P0 最小用例集

为避免第一轮实现范围过大，M0/M1 阶段先落地以下最小 P0 用例集。该集合不追求覆盖全部 method，而是优先打通协议、路由、Mock、任务、资源、usage、trace 和异常主链路。

### 6.1 M0 最小集

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

### 6.2 M1 最小集

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

## 7. 不可用时策略与兜底验收

规则以 [aicc_router.md §9](../../../aicc_router.md#9-fallback-与失败处理)、[§11](../../../aicc_router.md#11-用途表与配置层次)、[§12](../../../aicc_router.md#12-控制面设计) 为准；用例 ID 与 [cases_l1_routing_scheduler.md](cases_l1_routing_scheduler.md) §5 对应。

前置：

- 两个 Mock OpenAI 实例 `openai-mock-1`、`openai-mock-2` 提供同一个模型 `gpt-mock`（Model Driver 与 origin 相同），`openai-mock-2` 另有一个价格更高的 `gpt-mock-pro` 和一个没有价格的 `gpt-mock-noprice`。
- 用例在测试用户的用户层用 `routing.usecase.set {target: "user", usecase: "llm.code", model: "gpt-mock@openai-mock-1", on_unavailable: …}` 固定原模型；需要管理员层或锁时用 `target: "zone"`。
- 调用走 `helper.llm_chat`（逻辑名 `llm.code`）；Mock 按路径前缀注入失败，记录每个实例收到的请求数。
- cleanup：`clear: ["model", "on_unavailable"]` 清掉用户层，删除测试建的策略组和绑定，清除故障标记，关闭兜底 LLM（见 [03_environment_safety_and_cleanup.md](../03_environment_safety_and_cleanup.md)）。

| case id | 步骤 | 期望 |
|---|---|---|
| `l3_krpc_provider_5xx_no_policy_real_error` | 不设策略，`openai-mock-1` 返回 503，连续调用 10 次 | 每次失败，错误带 `openai-mock-1`、`gpt-mock@openai-mock-1` 和上游 503；`openai-mock-2` 收到 0 次请求；第 11 次 `route.resolve` 仍选 `gpt-mock@openai-mock-1` |
| `l3_krpc_provider_404_keeps_model` | 不设策略，`openai-mock-1` 返回 404 模型不存在，随后立刻再调用 | 两次都选 `gpt-mock@openai-mock-1`，返回真实 404 |
| `l3_krpc_provider_5xx_same_model_switch` | 设 `same_model`，`openai-mock-1` 返回 503 | `openai-mock-1` 收到 2 次（原请求 + 一次重试），`openai-mock-2` 收到 1 次；成功；`route_trace.fallback = {kind: "unavailable", from: "gpt-mock@openai-mock-1", to: "gpt-mock@openai-mock-2", policy: "same_model"}`；usage 一条，记在 `openai-mock-2`，`fallback_from = gpt-mock@openai-mock-1`；`events.list` 有一条 `unavailable_fallback` |
| `l3_krpc_unavailable_backup_fails` | 设 `same_model`，两个实例都返回 503 | 只切换一次，失败；`details.fallback.from` / `to` 分别给出两边的精确模型和错误 |
| `l3_krpc_unavailable_not_triggered` | 设 `next_model`，`openai-mock-1` 分别返回 400、401、403、409，以及返回带 usage 的失败 | 都不切换，返回原错误，`openai-mock-2` 收到 0 次请求 |
| `l3_krpc_unavailable_not_pricier` | 设 `not_pricier`；用 `openai-mock-2` 的 `instance_rules.enabled_inventory_models` 去掉它的 `gpt-mock`，只留 `gpt-mock-pro`（更贵）和 `gpt-mock-noprice`（无价格）；`openai-mock-1` 返回 503 | 不切换，返回原错误；用途表该行 `fallback_note = no_known_cheaper_model`；给 `gpt-mock-noprice` 补上不高于原模型的价格后再调用，切换到它 |
| `l3_krpc_unavailable_next_model` | 设 `next_model`，`openai-mock-1` 超时 | 停用 `openai-mock-1` 后重新路由，切到 `openai-mock-2` 上路由结果的第一个模型，trace `policy = next_model` |
| `l3_krpc_unavailable_request_off` | 设 `same_model`，请求带 `policy.no_fallback = true`，`openai-mock-1` 返回 503 | 不切换，返回原错误 |
| `l3_krpc_unavailable_exact_model` | 设 `same_model`，直接用 `chat.completions.create` 调 `gpt-mock@openai-mock-1`，`openai-mock-1` 返回 503 | 不切换（精确模型从不换模型） |
| `l3_krpc_unavailable_quota_denied` | 设 `same_model`，用额度规则拒绝测试用户使用 `openai-mock-1` | 原模型不发请求，切到 `gpt-mock@openai-mock-2`；不设策略时返回额度错误 |
| `l3_krpc_marked_faulty` | `provider.update {provider_instance_name: "openai-mock-1", marked_faulty: true}`；分别在设与不设 `same_model` 时调用；最后清除标记 | 设策略：`openai-mock-1` 收到 0 次请求，直接用 `openai-mock-2`；不设策略：立即返回 `no_provider_available`，`details.reason = marked_faulty`；`routing.usecases` 前后不变；`provider.list` 显示 `marked_faulty`；事件 `provider_marked_faulty` / `provider_fault_cleared`；清除后恢复调用 `openai-mock-1` |
| `l3_krpc_fallback_llm_coverage` | 只保留一个小模型（不满足 `llm.plan` 的门槛），调用 `llm.plan`；再由管理员 `routing.usecase.set {target: "zone", usecase: "llm.fallback", enabled: true}` 后重试 | 关闭时返回 `no_candidate_model`，`details.usecase = llm.plan`；打开后成功，`route_trace.fallback = {kind: "coverage", from: "llm.plan", to: "llm.fallback"}`，用途表该行 `coverage = fallback_llm`；该小模型的 Provider 返回 503 时不会再转到别处（除非 `llm.fallback` 行配了策略） |
| `l3_krpc_usecase_table` | 调 `routing.usecases`（`self`、`zone`、`group:<id>`、`app:<app_id>`），再用过期的 `revision` 调 `routing.usecase.set` | 29 行用途加 `llm.fallback`；每行模型、来源、锁、不可用时策略、`fallback_to` 正确；过期 revision 返回 `settings_revision_conflict`；普通用户看 `zone` 被拒绝 |
| `l3_krpc_routing_group` | 建策略组，组内把 `llm.code` 设为 `gpt-mock@openai-mock-2`，绑定到测试 App；分别从该 App、从别的 App、带 `route_group`、带别人的组 ID 调用 | 绑定 App 的调用选 `openai-mock-2`；别的 App 走用户层；`route_group` 优先于 App 绑定；别人的组被忽略且 trace 标出 `route_group_ignored`；删除组后绑定消失 |
| `l3_krpc_provider_validate_coverage` | 对一个新 Mock Provider 草案调 `provider.validate` | `usecase_coverage` 给出 29 个用途加入前后的模型与覆盖状态，未写入 system_config |
