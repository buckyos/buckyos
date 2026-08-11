# AICC 覆盖矩阵与需求追踪

定义功能覆盖矩阵和需求追踪矩阵，确保每个需求来源都能映射到稳定用例族和测试层级。

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
