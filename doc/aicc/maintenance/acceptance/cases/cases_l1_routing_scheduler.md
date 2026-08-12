# AICC L1 路由与调度用例

定义 L1 路由、逻辑目录、fallback、调度、request overlay、硬过滤和 trace 的 Rust 单测覆盖。

本文档是拆分后的自包含验收任务文档。实现或评审本任务时，以本文档和 README 中列出的依赖文档为准。

## 1. 功能覆盖矩阵

| 功能域 | 必测点 | 主要层级 |
|---|---|---|
| API 分层 | `route.resolve`（拒绝 exact model、返回 selected_exact_model/provider_options/fallback_attempts/enabled+disabled_capabilities/trace）、typed inference（`chat.completions.create`/`images.generate` 只接受 exact model、拒绝逻辑模型、不 fallback）、`helper.llm_chat`/`helper.text_to_image` 等价于 route+typed inference、legacy all-in-one 兼容 | L1/L2/L3 |
| 逻辑模型定义 | `min_line` admission 过滤、`disable_line` 禁用能力、`mount_mode` auto-mount、manual override | L1/L3 |
| Metadata resolver | exact→pattern→default→conservative 匹配优先级、unknown model 不默认高风险能力、metadata 缺失/损坏退回 builtin 可启动、variant 展开 + provider_options lowering | L1/L3 |
| Session overlay | `SessionLogicalProfile` inherit（可 fallback）/ replace（quota exhausted 失败）、overlay trace | L1/L3 |
| Method schema | `llm.chat`、`llm.completion`、`embedding.text`、`embedding.multimodal`、`rerank`、`image.*`、`vision.*`、`audio.*`、`video.*`、`agent.computer_use` 占位语义 | L1/L3/L4 |
| Provider inventory | `provider_instance_name`、`provider_type`、`provider_driver`、`exact_model`、`api_types`、`logical_mounts`、capabilities、pricing、health | L1/L3 |
| 路由解析 | 逻辑模型、精确模型、旧 alias 兼容、非法模型名、目录不存在 | L1/L2/L3 |
| Fallback | `strict`、`parent`、`target_exact`、`target_logical`、`disabled`、环路检测、最大深度 | L1/L3 |
| 调度 | `cost_first`、`latency_first`、`quality_first`、`balanced`、`local_first`、`strict_local`、权重优先、同权重 profile 评分 | L1 |
| Request Overlay | `session_overlay` 合并、逻辑目录覆盖、policy locked | L1/L3 |
| Task | 同步成功、异步 running、失败 task、cancel、无权限查询/取消、重复 idempotency | L1/L2/L3 |
| 资源 | `ResourceRef::Url`、`Base64`、`NamedObject`、FileObject meta、artifact 输出、大批量 embedding artifact | L1/L3/L4 |
| Streaming | Provider-native streaming 转最终 summary；中间态写 task data；AICC response 只返回 `succeeded` 或 `running` | L1/L3/L4 |
| Usage log | 成功调用写一条 durable event；幂等不重复写；缺 usage 视为 provider protocol error；按 1d/7d/provider/model 查询 | L1/L3 |
| 控制与管理 method | `cancel`、`reload_settings` / `service.reload_settings`、`models.list` / `service.models.list`、`usage.query`、`quota.query`、`provider.list`、`provider.health`、`provider.validate`、`provider.add`、`provider.delete`、`provider.refresh_models` | L1/L2/L3/L4 |
| 配置 | system_config 写入、全量/局部更新、Provider validate/add/delete/refresh、`reload_settings`、`models.list` 生效验证 | L3/L4 |
| 维护更新 | 模型事实基线、运营策略、`remote_cache` / 本地 override、随版本内置缓存、provider settings、routing_config、相关用例筛选、发布后复验、事实配置回滚、策略配置回滚 | L3/L4 |
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
| R-007 fallback 策略 | `routing_fallback_*` | L1/L3 |
| R-008 fallback 环路检测 | `routing_fallback_loop_*` | L1 |
| R-009 权重与 profile 调度 | `scheduler_weight_profile_*` | L1 |
| R-010 request overlay | `request_overlay_*` | L1/L3 |
| R-011 运行时 failover | `runtime_failover_*` | L1/L3/L4 |
| R-012 route trace | `trace_route_*` | L1/L3/L4 |
| R-013 配置化策略合并 | `route_overlay_merge_*` | L1 |
| R-014 精确模型默认不 fallback | `routing_exact_no_fallback_*` | L1/L2/L3 |
| R-015 目录 item 权重 | `scheduler_item_weight_*` | L1 |
| R-016 精确模型权重 | `scheduler_exact_model_weight_*` | L1 |
| R-017 应用侧 overlay 分层 | `request_overlay_layering_*` | L1/L3 |
| R-018 request overlay 继承与覆盖 | `request_overlay_inherit_patch_*` | L1 |
| R-019 目录软链接环检测 | `routing_logical_tree_loop_*` | L1 |
| R-020 删除 AICC 内部 session config 状态 | `request_overlay_stateless_*` | L1 |
| R-021 用户友好 trace summary | `trace_user_summary_*` | L1/L3/L4 |
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
| `l1_routing_exact_model_*` | P0 | 精确模型解析、Provider instance 校验、API type 校验、默认不 fallback |
| `l1_routing_logical_tree_*` | P0 | 逻辑目录展开、items target、目录软链接、候选去重 |
| `l1_routing_fallback_*` | P0 | `strict`、`parent`、`target_exact`、`target_logical`、`disabled` |
| `l1_routing_loop_*` | P0 | fallback loop、logical tree loop、最大 fallback depth |
| `l1_scheduler_weight_*` | P0 | item weight、exact model weight、weight 0 硬过滤、同权重 profile 评分 |
| `l1_scheduler_profile_*` | P0 | `cost_first`、`latency_first`、`quality_first`、`balanced`、`local_first`、`strict_local` |
| `l1_request_overlay_*` | P0 | overlay 合并、逻辑目录覆盖、policy locked、互不污染 |
| `l1_provider_protocol_openai_*` | P0 | OpenAI request/response 转换、tool call、JSON schema、SSE 聚合 |
| `l1_provider_protocol_claude_*` | P0 | Claude content block、tool use、vision block、stop reason、usage |
| `l1_provider_protocol_gemini_*` | P0 | Gemini parts、function call、safety block、operation 状态 |
| `l1_provider_protocol_fal_*` | P1 | fal submit/poll、artifact URL、operation timeout |
| `l1_resource_ref_*` | P0 | `url`、`base64`、`named_object`、FileObject meta 推导 |
| `l1_task_lifecycle_*` | P0 | immediate、async running、final succeeded、failed、cancel |
| `l1_usage_log_*` | P0 | 成功写 usage、幂等去重、缺 usage 报错、查询聚合 |
| `l1_method_api_type_canonical_*` | P0 | `method` 与 `api_type` 边界、`llm` vs `llm.chat`、非正式 api_type 拒绝或降级诊断 |
| `l1_control_method_*` | P0 | cancel、reload、models list、usage/quota/provider 查询的 schema 和权限边界 |
| `l1_security_*` | P0 | `local_only`、`proxy_unknown`、locked policy、trace 脱敏 |
| `l1_concurrency_*` | P1 | session patch 并发、幂等并发、异步任务并发完成 |

### 3.2 L2 AiccClient 黑盒测试

| 用例族 | 优先级 | 覆盖点 |
|---|---|---|
| `l2_client_llm_chat_success` | P0 | AiccClient 构造标准 `llm.chat` 请求并解析成功响应 |
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
| `l3_provider_admin_*` | P0 | provider.validate/add/delete/refresh_models 的 system_config 写入、reload 和回滚语义 |
| `l3_models_list_*` | P0 | `models.list` / `service.models.list` inventory、逻辑目录、health、legacy aliases 脱敏诊断 |
| `l3_quota_query_*` | P1 | `quota.query` 按 tenant、capability、method 返回预算状态和拒绝路径 |
| `l3_krpc_llm_chat_*` | P0 | 纯文本、多模态 content part、tool call、JSON schema |
| `l3_krpc_resource_*` | P0 | `url`、`base64`、`named_object` 输入和 artifact 输出 |
| `l3_krpc_stream_*` | P0 | Mock streaming chunks、task data progress、final summary |
| `l3_krpc_async_*` | P0 | image/audio/video 类异步 task 状态闭环 |
| `l3_krpc_usage_*` | P0 | usage event 写入和查询 |
| `l3_krpc_failover_*` | P0 | Provider timeout / 5xx / quota exhausted 后 failover |
| `l3_krpc_security_*` | P0 | local_only、跨用户访问拒绝、脱敏扫描 |
| `l3_krpc_legacy_*` | P1 | legacy alias、旧字段兼容或迁移提示 |

### 3.4 L4 Gateway 真实模型验收

| 用例族 | 优先级 | 覆盖点 |
|---|---|---|
| `l4_gateway_openai_<model>_complex_workflow` | P2 | OpenAI 每个支持模型的文本、JSON schema、tool call、usage、trace |
| `l4_gateway_claude_<model>_complex_workflow` | P2 | Claude 每个支持模型的多模态或 vision、tool use、usage、trace |
| `l4_gateway_gemini_<model>_complex_workflow` | P2 | Google Gemini 每个支持模型的多模态、safety / function call / operation 语义 |
| `l4_gateway_openrouter_<model>_complex_workflow` | P2 | OpenRouter 每个支持模型的 OpenAI-compatible 协议兼容、usage、trace |
| `l4_gateway_fal_<model>_media_workflow` | P2 | fal 每个支持模型的 image/video/audio 工具型异步任务和 artifact |
| `l4_gateway_sn_ai_provider_<model>_complex_workflow` | P2 | SN AI Provider 每个支持模型的无普通 API key 链路、usage、trace、provider 归因 |
| `l4_gateway_models_list` | P2 | 真实环境 inventory、逻辑目录和 Provider health 可诊断 |

L4 用例 ID 中的 `<model>` 必须使用稳定可读的 slug，由精确模型名归一化得到；报告中必须保留原始精确模型名。

## 4. 首批 P0 最小用例集

为避免第一轮实现范围过大，M0/M1 阶段先落地以下最小 P0 用例集。该集合不追求覆盖全部 method，而是优先打通协议、路由、Mock、任务、资源、usage、trace 和异常主链路。

### 4.1 M0 最小集

| case id | 层级 | 目标 |
|---|---|---|
| `l1_routing_exact_model_success` | L1 | 精确模型名解析成功 |
| `l1_routing_exact_model_no_fallback` | L1 | 精确模型不可用且未开启 fallback 时失败 |
| `l1_routing_logical_model_candidates` | L1 | 逻辑模型展开候选列表 |
| `l1_routing_parent_fallback_success` | L1 | parent fallback 生效 |
| `l1_routing_fallback_loop_rejected` | L1 | fallback 环路被拒绝 |
| `l1_scheduler_weight_priority` | L1 | 目录 item weight 优先级生效 |
| `l1_scheduler_profile_cost_first` | L1 | 同优先级候选按 cost profile 选择 |
| `l1_request_overlay_override_route` | L1 | request overlay 覆盖系统配置并改变最终物理路由 |
| `l1_request_overlay_stateless` | L1 | 不同 request overlay 互不污染，AICC 不保存 session config |
| `l1_security_local_only_rejects_cloud` | L1 | `local_only` 硬过滤云端 Provider |
| `l1_provider_openai_chat_success` | L1 | OpenAI-like `llm.chat` 协议转换成功 |
| `l1_provider_openai_stream_merge` | L1 | Provider streaming chunks 聚合为最终 summary |
| `l1_resource_ref_json_tags` | L1 | `url`、`base64`、`named_object` JSON tag 正确 |
| `l1_task_immediate_success` | L1 | 同步成功任务写入 result |
| `l1_task_async_success` | L1 | 异步任务 running 到 succeeded 闭环 |
| `l1_usage_success_write_once` | L1 | 成功调用写入 exactly one usage event |
| `l1_usage_missing_usage_rejected` | L1 | 成功响应缺 usage 被判为协议错误 |
| `l2_client_llm_chat_success` | L2 | AiccClient 调用 `llm.chat` 成功 |
| `l2_client_idempotency_conflict` | L2 | 同 key 不同 body 返回 idempotency conflict |
| `l2_client_cancel_unknown_task` | L2 | 取消不存在任务返回可判断错误 |

### 4.2 M1 最小集

| case id | 层级 | 目标 |
|---|---|---|
| `l3_settings_reload_mock_openai` | L3 | 写入 Mock settings 后 reload 生效 |
| `l3_models_list_mock_inventory` | L3 | `models.list` 可看到 Mock Provider inventory |
| `l3_krpc_llm_chat_text_success` | L3 | kRPC `llm.chat` 纯文本成功 |
| `l3_krpc_llm_chat_json_schema_success` | L3 | JSON schema 输出可解析 |
| `l3_krpc_resource_base64_image` | L3 | base64 图片资源输入成功 |
| `l3_krpc_resource_named_object_artifact` | L3 | named_object artifact 输出可读取 |
| `l3_krpc_stream_progress_and_final` | L3 | streaming 中间态写 task data，最终 summary 正确 |
| `l3_krpc_async_task_success` | L3 | 异步任务状态闭环 |
| `l3_krpc_provider_5xx_failover` | L3 | Provider 5xx 后按策略 failover |
| `l3_krpc_provider_timeout_failed` | L3 | Provider timeout 返回明确错误 |
| `l3_krpc_usage_query_last_1d` | L3 | usage 可按 last_1d 查询 |
| `l3_krpc_security_no_secret_in_report` | L3 | 报告和 trace 脱敏扫描通过 |

首批 P0 最小集通过后，再扩展到完整 P0/P1/P2 用例矩阵。

