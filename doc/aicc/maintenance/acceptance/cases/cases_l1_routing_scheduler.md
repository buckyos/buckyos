# AICC L1 路由与调度用例

定义 L1 路由、逻辑目录、用途表、不可用时策略与兜底 LLM、调度、request 策略、硬过滤和 trace 的 Rust 单测覆盖。

本文档是拆分后的自包含验收任务文档。实现或评审本任务时，以本文档和 README 中列出的依赖文档为准。

## 1. 功能覆盖矩阵

| 功能域 | 必测点 | 主要层级 |
|---|---|---|
| API 分层 | `route.resolve` 返回完整渠道/模型身份、operation 和 trace（不再有 `fallback_attempts`）；typed inference 只接受 exact model，从不换模型（包括用 `route.resolve` 得到的精确模型）；`helper.*` 按用途路由，只有它会执行用途的不可用时策略 | L1/L2/L3 |
| 逻辑模型定义 | `min_line` admission 过滤、`disable_line` 禁用能力、`mount_mode` auto-mount、manual override | L1/L3 |
| Catalog resolver | Provider mapping 优先、跨 Model Driver 唯一匹配、exact→ordered pattern→default、unknown conservative fallback、variant + Provider Rules lowering | L1/L3 |
| 用途表 | 29 个用途加兜底 LLM 一行；各层合并（默认 < 管理员 < 用户 < App 组 < 会话组）、管理员锁定、层选的模型不可用时跟随下一层、`routing.usecases` 的视角与 `without_provider`、`routing.usecase.set` 的校验与版本冲突、策略组与 App 绑定、请求 `route_group`（不属于调用者的组被忽略并在 trace 标出） | L1/L3 |
| Method schema | `chat.completions.create`、`embedding.text`、`embedding.multimodal`、`rerank`、`image.*`、`vision.*`、`audio.*`、`video.*`、`agent.computer_use` 占位语义 | L1/L3/L4 |
| Provider inventory | Instance/Profile/adapter、ModelUID/Driver/origin、原始 provider model、operation、capabilities、pricing source、health | L1/L3 |
| 路由解析 | 逻辑模型、精确模型、非法模型名、目录不存在 | L1/L2/L3 |
| 不可用时与兜底 | `on_unavailable` 四个值、触发与不触发类别、每次请求最多切换一次、备选沿用原请求与用途约束、标记为故障、兜底 LLM（默认关闭，只为静态无候选的 LLM 用途提供，Provider 故障不转到它）；注册表没有 fallback 链 | L1/L3 |
| 调度 | `latency_first`、`quality_first`、`balanced`（默认）、`local_first`、`strict_local`、权重优先、同权重 profile 评分；健康、熔断、冷却、延迟观测不参与选择（只有请求显式 `max_latency_ms` 是门槛） | L1 |
| Request 策略 | 只能收紧：`no_fallback`、`local_only`、屏蔽 Provider、允许列表与管理员的取交集、降低成本上限与延迟门槛；未锁定时可换 profile；`session_overlay`、`allow_fallback`、`runtime_failover`、profile `cheap` 被拒绝 | L1/L3 |
| Task | 同步成功、异步 running、失败 task、cancel、无权限查询/取消、重复 idempotency | L1/L2/L3 |
| 资源 | `ResourceRef::Url`、`Base64`、`NamedObject`、FileObject meta、artifact 输出、大批量 embedding artifact | L1/L3/L4 |
| Streaming | Provider-native streaming 转最终 summary；中间态写 task data；AICC response 只返回 `succeeded` 或 `running` | L1/L3/L4 |
| Usage log | 成功调用写一条 durable event；幂等不重复写；缺 usage 视为 provider protocol error；按 1d/7d/provider/model 查询 | L1/L3 |
| 控制与管理 method | `cancel`、`service.reload_settings`、`models.list`、`usage.query`、`quota.query`、`provider.list`、`provider.health`、`provider.validate`（含 `usecase_coverage`）、`provider.add`、`provider.update`（含 `marked_faulty`）、`provider.delete`、`provider.refresh_models`、`provider.catalog`、`protocol_adapter.list`、`driver_metadata_update.get/set`、`routing.usecases`、`routing.usecase.set`、`routing.groups.*`、`routing.bindings.*` | 历史 L1/L2/L3/L4；发布门禁按 T1/T1.5/T2/T3 |
| 配置 | system_config 写入、全量/局部更新、Provider validate/add/delete/refresh、`service.reload_settings`、`models.list` 生效，以及 Provider 库存定时循环停止事件与优雅退出验证 | L3/L4 |
| 维护更新 | 模型事实基线、运营策略、NDN target seq、Provider applied/updating seq、model 列表变化矩阵、本地 override、双触发点全局收敛和发布后复验 | L3/L4 |
| 安全 | `local_only` 硬过滤、`proxy_unknown` 非本地、trace 脱敏、密钥不入日志、跨租户隔离 | L1/L3/L4 |

## 2. 需求追踪矩阵

| 需求来源 | 覆盖用例族 | 层级 |
|---|---|---|
| `aicc_router.md` R-001 精确模型名解析 | `routing_exact_model_*` | L1/L2/L3 |
| R-002 逻辑模型目录树 | `routing_logical_tree_*` | L1/L3 |
| R-003 多 Provider 挂载 | `routing_multi_provider_*` | L1/L3 |
| R-004 Provider 声明式元数据 | `provider_inventory_*` | L1/L3 |
| R-005 候选列表生成 | `routing_candidates_*` | L1 |
| R-006 硬性过滤 | `routing_hard_filter_*` | L1/L3 |
| `aicc_router.md` §9.3 不可用时策略（取代 R-007） | `routing_fallback_*` | L1/L3 |
| §9.3 每次请求最多切换一次（取代 R-008；没有 fallback 链，不再有环路） | `routing_fallback_once_*` | L1 |
| R-009 权重与 profile 调度 | `scheduler_weight_profile_*` | L1 |
| §12.3 Request 级策略只能收紧（取代 R-010；请求不再携带 `session_overlay`） | `request_policy_*` | L1/L3 |
| §9.2 没有策略时不换模型（取代 R-011 运行时 failover） | `routing_no_switch_*` | L1/L3/L4 |
| R-012 route trace | `trace_route_*` | L1/L3/L4 |
| §11.2 用途表的层次与锁定（取代 R-013） | `usecase_layers_*` | L1 |
| R-014 精确模型从不换模型 | `routing_exact_no_fallback_*` | L1/L2/L3 |
| R-015 目录 item 权重 | `scheduler_item_weight_*` | L1 |
| R-016 精确模型权重 | `scheduler_exact_model_weight_*` | L1 |
| §12.2 路由策略组与 scope（取代 R-017） | `routing_group_*` | L1/L3 |
| §11.3 求值：层选的模型不可用时跟随下一层（取代 R-018） | `usecase_resolve_*` | L1 |
| R-019 目录软链接环检测 | `routing_logical_tree_loop_*` | L1 |
| §11.5 稳定性契约：健康、熔断、延迟观测不改变路由（取代 R-020） | `routing_stability_*` | L1/L3 |
| R-021 用户友好 trace summary | `trace_user_summary_*` | L1/L3/L4 |
| §9.5 兜底 LLM | `routing_fallback_llm_*` | L1/L3 |
| §9.6 手工标记故障 | `routing_marked_faulty_*` | L1/L3 |
| §11.4 覆盖与添加 Provider | `usecase_coverage_*` | L1/L3 |
| `aicc_api设计.md` ResourceRef | `resource_ref_*` | L1/L3 |
| `aicc_api设计.md` idempotency | `idempotency_*` | L1/L2/L3 |
| `aicc_api设计.md` method / api_type 命名 | `method_api_type_canonical_*` | L1/L2/L3 |
| `aicc_api设计.md` control method | `control_method_*` | L1/L2/L3 |
| AICC 服务管理 method | `provider_admin_*`、`models_list_*`、`quota_query_*`、`usage_query_*` | L2/L3/L4 |
| `aicc_usage_log_db_requirements.md` usage event | `usage_log_*` | L1/L3 |
| `maintenance/update_aicc_settings_via_system_config.md` reload | `settings_reload_*` | L3/L4 |
| `maintenance/aicc_maintenance_roles.md` 统一更新验收流程 | `maintenance_update_*` | L3/L4 |
| `maintenance/aicc_maintenance_roles.md` 模型事实 / 运营策略分离 | `maintenance_metadata_*`、`maintenance_policy_*` | L3/L4 |
| `maintenance/aicc_maintenance_roles.md` 服务商 / 用户配置覆盖 | `maintenance_provider_settings_*`、`maintenance_routing_config_*` | L3/L4 |
| `maintenance/aicc_maintenance_roles.md` 回滚验收 | `maintenance_rollback_*` | L3/L4 |

## 3. 分层用例清单

本节把前文功能域拆成更接近实现任务的用例清单。用例 ID 可在实现时继续细化，但应保持前缀稳定。

### 3.1 L1 白盒单测

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

### 3.2 L2 AiccClient 黑盒测试

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

### 3.3 L3 本地 kRPC 黑盒测试

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

### 3.4 L4 Gateway 真实模型验收

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

## 4. 首批 P0 最小用例集

为避免第一轮实现范围过大，M0/M1 阶段先落地以下最小 P0 用例集。该集合不追求覆盖全部 method，而是优先打通协议、路由、Mock、任务、资源、usage、trace 和异常主链路。

### 4.1 M0 最小集

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

### 4.2 M1 最小集

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

## 5. 用途表、不可用时策略与兜底用例

本节是 2026-11-27 用途表改造后的路由验收清单，规则以 [aicc_router.md §9](../../../aicc_router.md#9-fallback-与失败处理)、[§11](../../../aicc_router.md#11-用途表与配置层次)、[§12](../../../aicc_router.md#12-控制面设计) 为准。L1 用例在 `src/frame/aicc` 的单测里实现（求值在 `routing/usecase.rs`，失败分类在 `execution/mod.rs` 的 `unavailable_trigger`）；L3 用例用 Mock Provider 走本地 kRPC，做法见 [cases_l3_local_p0_core.md](cases_l3_local_p0_core.md) §7。

已删除、不再验收的旧行为：`strict` / `parent` / `target_exact` / `target_logical` / `disabled` 五种 fallback 模式、fallback 环路与深度、`runtime_failover` 与执行期候选循环、请求 `allow_fallback` / `session_overlay` / `SessionLogicalProfile`、`route.resolve` 的 `fallback_attempts`、`cost_first` 与请求 profile `cheap`、健康状态 / 熔断 / 冷却改变路由。

### 5.1 稳定性与不切换

| case id | 层级 | 目标 |
|---|---|---|
| `l1_routing_no_policy_no_switch` | L1/L3 | 用途没有不可用时策略（默认 `none`）：同池两个 Provider，选中者连续失败 10 次，另一个收到 0 次请求；每次返回上游真实错误（Provider 实例名、精确模型、状态码），不改写成 `no_candidate_model` |
| `l1_routing_404_keeps_model` | L1/L3 | 选中模型返回 404 模型不存在后立刻再请求，仍选同一模型 |
| `l1_routing_stability_health_ignored` | L1 | 健康状态、熔断、冷却、观测延迟、上游配额余量都不改变选中模型；只有请求显式 `max_latency_ms` 按 p95 过滤。请求的 token 数、价格和会话历史也不改变池内排序（router §10.4）；同分候选按默认顺序 |
| `l1_routing_stability_default_upgrade` | L1/L3 | Provider 刷新出权重更高的新版本模型时默认路由随之升级；同一 Provider 连续失败（未标记故障）时默认路由不变 |
| `l1_routing_exact_model_no_fallback` | L1/L3 | 精确模型请求（含 `route.resolve` 得到的精确模型再走 typed 调用）从不切换，即使该用途配置了策略 |
| `l1_routing_spec_path_no_policy` | L1 | 规格、家族路径（如 `llm.gpt-mini`）按静态展开，不读用途配置，不执行不可用时策略 |

### 5.2 不可用时策略

| case id | 层级 | 目标 |
|---|---|---|
| `l1_routing_fallback_trigger_classes` | L1 | 触发：同模型重试后仍 timeout / 5xx / 连接失败、模型不存在或下线、上游配额耗尽（402 等）、用户额度规则拒绝该模型、Provider 被标记为故障；不触发：400 / 401 / 403 / 409、取消、已产生用量的失败、异步任务拿到 remote task ID 之后的失败。每类一个断言 |
| `l1_routing_fallback_retry_first` | L1/L3 | 切换前先对原模型重试一次（`Retry-After` 最多等 2 秒）并允许同一 Provider 重新登录；Provider 调用次数 = 原模型 + 一次重试 + 备选 |
| `l1_routing_fallback_same_model` | L1/L3 | `same_model`：备选是另一个 Provider 实例上 `model_driver_id`、`origin_model_id`、variant 都相同的精确模型，按 Provider 权重降序、再按配置顺序；只接了一个 Provider 时没有备选，返回原错误 |
| `l1_routing_fallback_not_pricier` | L1/L3 | `not_pricier`：备选的每个计费维度单价都已知且不高于原模型（按 catalog 折成 USD）才接受；更贵或价格未知的被排除后再算；原模型价格未知时没有备选；补上价格后切换 |
| `l1_routing_fallback_next_model` | L1/L3 | `next_model`：把原 Provider 视作停用重新路由，取第一个结果 |
| `l1_routing_fallback_once` | L1/L3 | 每次请求最多切换一次；备选也失败时返回错误，`details.fallback.from` / `to` 同时给出两边的精确模型和原因 |
| `l1_routing_fallback_constraints` | L1 | 备选同样满足原请求与用途的能力、隐私、`local_only`、可见性要求，并重新做额度准入；不跨 API 类型 |
| `l1_routing_fallback_no_fallback_request` | L1/L3 | 请求 `policy.no_fallback = true` 时策略不生效，返回原错误 |
| `l1_routing_fallback_visible` | L1/L3 | 发生切换时 `route_trace.fallback = {kind: "unavailable", from, to, policy}`、`user_summary.was_failover = true`；usage 只记在实际执行的模型上，带 `fallback_from`，`usage.query` 的 `fallback_only` 能筛出；事件 `unavailable_fallback` 带 from、to、policy、reason、usecase、task_id |
| `l1_routing_fallback_async_before_task_id` | L1 | 异步任务（视频等）只在拿到 remote task ID 之前允许切换 |

### 5.3 兜底 LLM 与标记故障

| case id | 层级 | 目标 |
|---|---|---|
| `l1_routing_fallback_llm_off` | L1/L3 | 兜底 LLM 默认关闭：没有静态候选的用途返回 `no_candidate_model`，`details.usecase` 给出用途名；不查父目录、`llm.fallback` 或其它用途 |
| `l1_routing_fallback_llm_coverage` | L1/L3 | 管理员打开兜底 LLM（管理员层 `usecases."llm.fallback".enabled = true`）后，七个 LLM 用途中静态无候选的由 `llm.fallback` 提供：`route_trace.fallback = {kind: "coverage", from: <用途>, to: "llm.fallback"}`、`user_summary.was_fallback = true`；只带请求自身的硬要求，不带用途的门槛；已覆盖的用途不受影响；非 LLM 用途不兜底 |
| `l1_routing_fallback_llm_not_on_failure` | L1/L3 | 用途有模型但 Provider 失败时不转到兜底 LLM（走 §5.2 的策略或返回原错误）；只因上下文不够而无候选时也不兜底 |
| `l1_routing_fallback_llm_own_row` | L1 | 经兜底提供的请求使用 `llm.fallback` 这一行的模型与不可用时策略；请求可以直接使用 `llm.fallback` |
| `l1_routing_marked_faulty_policy` | L1/L3 | `provider.update {marked_faulty: true}` 后，选中模型在该 Provider 上且用途有策略时不发请求、直接用备选（reason `marked_faulty`）；算备选时排除所有被标记的 Provider |
| `l1_routing_marked_faulty_no_policy` | L1/L3 | 没有策略的用途立即返回 `no_provider_available`，`details = {reason: "marked_faulty", provider_instance_name, exact_model}`，Mock 收到 0 次请求 |
| `l1_routing_marked_faulty_table_unchanged` | L1/L3 | 标记前后 `routing.usecases` 的模型与来源不变；只改标记时不重新校验 Provider；清除标记后恢复调用；事件 `provider_marked_faulty` / `provider_fault_cleared` 各一条 |

### 5.4 用途表、层次与策略组

| case id | 层级 | 目标 |
|---|---|---|
| `l1_usecase_layer_override_route` | L1/L3 | 每一格（模型、不可用时）由设了它的最高一层决定：默认 < 管理员 < 用户 < App 绑定的组 < 会话组；`routing.usecases` 每行给出 `model_source` 与 `on_unavailable_source` |
| `l1_usecase_layers_lock` | L1/L3 | 管理员层 `locked` 的格子更高层不能改；锁定的模型本身不可用时使用默认展开；非管理员层写 `locked` 被拒绝 |
| `l1_usecase_layers_follow_next` | L1/L3 | 层选的模型缺失 / 不可见 / 不满足用途要求时跟随下一层，trace `usecase.skipped_models` 与用途表 `layers[].model_state`（`missing` / `not_permitted` / `unsuitable`）标出；管理员层的模型从库存消失时事件 `usecase_model_missing` 只记一次，回来时记 `usecase_model_recovered`，配置保留 |
| `l1_usecase_set_validation` | L1/L3 | `routing.usecase.set` 校验：用途存在；模型是库存里的精确模型 `model@provider`、API 类型一致、满足用途要求（`llm.fallback` 不查门槛）、对调用者可见；`enabled` 只能写在管理员层的 `llm.fallback`；`revision` 不符返回 `settings_revision_conflict`；`clear` 恢复跟随下一层 |
| `l1_usecase_table_views` | L3 | `routing.usecases` 的 `view`：`self`（默认）、`zone`（只给管理员）、`group:<id>`、`app:<app_id>`；`revision` 与 `can_edit` 对应该视角；29 个用途加 `llm.fallback` 一行；`fallback_to` / `fallback_note` 预测当前模型的 Provider 不可用时换到谁 |
| `l1_usecase_without_provider` | L3 | 管理员用 `without_provider` 预演删除或停用某个 Provider：与正常用途表比较，列出由谁接替、哪些变成未覆盖；普通用户调用被拒绝 |
| `l1_routing_group_app_binding` | L1/L3 | 策略组绑定到 App（Agent 的 app_id 是 agent_id）后，该用户经这个 App 的调用生效；别的用户经同一个 App 不受影响；删除组时绑定一起删除 |
| `l1_routing_group_session` | L1/L3 | 请求 `route_group` 生效且优先于 App 绑定；组不属于调用者的额度用户时被忽略，trace `usecase.route_group_ignored = true`，不报错 |
| `l1_request_policy_tighten_only` | L1/L3 | 请求策略只能收紧：`blocked_provider_instances` 追加、`allowed_provider_instances` 与管理员的列表取交集、`max_cost` 只能降低（同币种）、`max_latency_ms` 只能降低、`local_only` 只能打开；`profile`（`fast` / `balanced` / `quality`）只在管理员未锁定时生效；请求带 `allow_fallback`、`runtime_failover`、`session_overlay` 或 profile `cheap` 时被拒绝 |
| `l1_scheduler_profile_balanced_default` | L1 | 管理员和请求都没有指定 profile 时按 `balanced`；`decision` 等原来用 `cost_first` 的内置定义改为 `balanced` |

### 5.5 覆盖与添加 Provider

| case id | 层级 | 目标 |
|---|---|---|
| `l1_usecase_coverage_validate` | L3 | `provider.validate` 返回 `usecase_coverage`：整个 Zone 29 个用途在加入该 Provider 前后的模型与覆盖状态（`covered` / `fallback_llm` / `uncovered`）；不写入 system_config |
| `l1_usecase_coverage_reason` | L1/L3 | 未覆盖的行给出原因：`no_model_for_api_type`（没有该 API 类型的模型）或 `requirements_unmet`（有模型但不满足用途要求）；管理员视角带路由错误 `detail` |
| `l1_usecase_coverage_keep_old` | L3 | 添加 Provider 时不接受某条“默认模型会变”的变化，用 `routing.usecase.set {target: "zone"}` 把旧模型写进管理员层；加入后该用途保持旧模型，`model_source = zone` |
