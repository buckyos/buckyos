# AICC 覆盖矩阵与需求追踪

定义功能覆盖矩阵和需求追踪矩阵，确保每个需求来源都能映射到稳定用例族和测试层级。

本文档是拆分后的自包含验收任务文档。实现或评审本任务时，以本文档和 README 中列出的依赖文档为准。

## 1. 功能覆盖矩阵

| 功能域 | 必测点 | 主要层级 |
|---|---|---|
| API 分层 | `route.resolve` 拒绝 exact model并返回完整渠道/模型身份、operation 和 trace（不再有 `fallback_attempts`）；全部 typed inference 只接受 exact model，从不换模型（包括用 `route.resolve` 得到的精确模型）；`helper.*` 按用途路由，只有它会执行用途的不可用时策略 | L1/L2/L3 |
| 逻辑模型定义 | `min_line` admission 过滤、`disable_line` 禁用能力、`mount_mode` auto-mount、manual override | L1/L3 |
| Catalog resolver | Provider mapping 优先、跨 Model Driver 唯一匹配、exact→ordered pattern→default、unknown conservative fallback、variant + Provider Rules lowering | L1/L3 |
| 用途表 | 29 个用途加兜底 LLM 一行；各层合并（默认 < 管理员 < 用户 < App 组 < 会话组）、管理员锁定、层选的模型不可用时跟随下一层、`routing.usecases` 的视角与 `without_provider`、`routing.usecase.set` 的校验与版本冲突、策略组与 App 绑定、请求 `route_group`（不属于调用者的组被忽略并在 trace 标出） | L1/L3 |
| Method schema | `chat.completions.create`、`embedding.*`、`rerank`、`images.generate`、`image.*`、`vision.*`、`audio.*`、`video.*`、`agent.computer_use` 占位语义 | L1/L3/L4 |
| Provider inventory | `provider_instance_name`、`provider_profile_id`、`protocol_adapter_id`、`model_uid`、`model_driver_id`、`origin_model_id`、原始 `provider_model_id`、operation、capabilities、pricing source、health | L1/L3 |
| 路由解析 | 逻辑模型、精确模型、非法模型名、目录不存在、完整身份链 | L1/L2/L3 |
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
