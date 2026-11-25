# AICC 去掉自动换模型、改为显式 fallback TODO

日期：2026-11-23。状态：立项，代码未实施。文档已按新原则修订（§6）。

规范以 [aicc_router.md §9](../doc/aicc/aicc_router.md#9-fallback-与失败处理) 为准，本文只写现状差距、步骤和待确认项。代码行号基于 `6c80c119c`，动手前先复核。

## 1. 原则

owner 2026-11-23 定：

1. 自动路由会让费用失控。系统引导用户配置正确的路由，建立稳定的预期。
2. 个人用户的配置大概率只解决“能不能用”，出问题没有可回退的 Provider；企业配多个 Provider 通常是成本考虑，很少接受一个功能点今天用这家、明天用那家。
3. Provider 不可用应该去修 Provider，不是自动跳到另一个。
4. fallback 是系统管理员明确手工配置的模型路由。目标通常比原模型便宜，但 Provider 里没有价格信息，只能由完成配置的系统管理员判断。

## 2. 现状：哪些地方还在自动换

| # | 行为 | 位置 | 说明 |
|---|---|---|---|
| G1 | 执行失败后切到同池其它候选 | `execution/mod.rs:1637`、`:1830`、`:1849`；`service/inference.rs:1159-1214`；`routing/policy.rs:97`（默认 `true`） | 池内是同权重的其它 Provider 或其它家族 |
| G2 | 健康状态决定选谁 | `routing/mod.rs` `hard_filter_runtime`（`model_available`、`health`、`credential_available`）；`health.rs`（404 冷却 5 分钟、连续 3 次失败熔断 30 秒） | **影响最大**：冷却或熔断后，后续请求落到低权重组。例如 `llm.code`：gpt-codex 3.2 → gpt-standard 3.0 → claude-opus 2.4 → …。只配 SN 一家也会发生，因为 SN 里有多家模型 |
| G3 | 内置默认 fallback | `service/model_defaults.rs:966` `llm_usecase_fallback()`（七个 LLM 用途 → `llm.fallback`）、`:1143` 起 `llm.fallback` 的 38 个规格 | 2026-09-25 曾改成“默认空、不回退”，2026-11-17 的 `059d3813a` 又加了回来 |
| G4 | 非 LLM 默认 `parent` | `service/model_defaults.rs:945` `parent_fallback()` 的各处调用（image、video、agent 等） | 用户选 `image.txt2img.flux`，没有候选时会扩大到 `image.txt2img` 下的任意家族 |
| G5 | 额度拒绝后换候选 | `execution/mod.rs:1830`（quota failover）；`service/inference.rs:1159` 起（precheck 被拒后提升下一个候选） | |
| G6 | 请求方可以打开换模型、指定目标 | `aicc_client.rs` `RoutePolicy.runtime_failover` / `allow_exact_model_fallback`、session overlay 的 `fallback`；websdk `aicc_client.ts:159`、`:175` | |
| G7 | 调用方默认显式打开 | `aiworkspace/aiws/host.js:31` `MEDIA_POLICY`；`tools/buckyos-agent/lib/aicc.ts:144-145` | |
| G8 | Provider 故障被报成“没有可用模型” | G2 的结果：候选被运行状态过滤空后返回 `no_candidate_model` | 单 Provider 用户看不到真实原因 |
| G9 | 没有配置 fallback 的界面 | AI Center 的 `RoutingPage.tsx` 只显示 fallback 链，不能配置；CLI 也没有 | 现在只能手写 `settings.routing_config` |

## 3. 任务

按顺序做，每步单独可合入。T1–T3 做完就不会再自动换；T4 以后补齐配置与可见性。

### T1 去掉执行期切换（G1、G5）

- [ ] 执行引擎只跑选中的模型：同模型重试保留；去掉候选循环和 quota failover。
- [ ] `inference.rs` 不再准备 `failover` 列表；precheck 被拒直接返回拒绝。
- [ ] 删除 `runtime_failover`（API、policy、overlay、websdk 类型），按 breaking change 处理，不留兼容；`route.resolve` 的 `fallback_attempts` 在 T4 之前恒为空。
- [ ] 凭证失效后重新登录一次属于修同一个 Provider，保留（PR #751 的这部分可以复用，见 §4）。

### T2 健康状态不参与选择（G2、G8）

- [ ] 把硬过滤拆成静态条件与运行状态：`enabled`、凭据**已配置**、能力、权限、隐私、预算属静态；`model_available` 冷却、熔断、`health`、动态登录失败、上游配额余量属运行状态。只有静态条件参与展开和过滤。
- [ ] 选中的模型处于故障时照常发请求，返回上游真实错误，带 Provider 实例名、精确模型、上游状态码与原文。
- [ ] `no_candidate_model` 只表示没有接入满足请求的模型。
- [ ] 一次成功或管理员修改该 Provider 配置后立即清除故障标记（现在的冷却是进程级的，改配置不清）。
- [ ] 请求显式给出 `max_latency_ms` 时仍作为该请求的硬门槛。

### T3 去掉内置 fallback（G3、G4）

- [ ] 七个 LLM 用途改回 strict，`llm.fallback` 清空；非 LLM 的 `parent` 全部改 strict；移除 `parent` 模式（Q5）。
- [ ] 同步 `model_defaults.rs` 头部的目录契约注释（有测试逐项比对）。
- [ ] 处理覆盖率缺口（Q2）：11-16 那轮加 fallback 的目的之一，是“只接入一个模型时七个用途都能用”。去掉后，缺口要由引导配置补上，不能让用户只看到一句“没有可用模型”。

### T4 显式 fallback（G6、G7、G9）

- [ ] 系统级配置：逻辑目录节点的 `fallback: {mode: target_exact | target_logical, target}`，只认 system 层；请求与 session overlay 只能 `disabled`。校验：目标存在、同 API namespace、不成环、不指向自己。
- [ ] 执行：触发条件与“每次请求最多一次”按 router §9.2、§9.3；失败时错误里同时给出原模型与 fallback 模型的原因。
- [ ] AI Center 路由页：为某个用途选择 fallback 模型。页面说明“系统不知道价格，请确认它比原模型便宜”，保存前要求确认；列出当前所有已配置的 fallback。
- [ ] CLI：查看、设置、清除。
- [ ] `host.js`、buckyos-agent 去掉默认打开的 policy；`--no-fallback` 只对应 `allow_fallback: false`。

### T5 让 Provider 故障看得见

- [ ] 连续失败或模型 404 时通知管理员（System Notify），同一故障恢复前只通知一次；Provider 页列出精确模型、上游错误、首次发生时间。
- [ ] 模型在上游目录里但调用 404：标成“目录中有、调用失败”，引导去核对渠道配置。
- [ ] 发生显式 fallback：route trace、用量记录和 AI Center 标出原模型、实际模型、原因，并提示去修原 Provider。
- [ ] 引导配置：接入 Provider 后，AI Center 列出哪些用途还没有可用模型，由用户确认后写入路由配置。

### T6 测试与手册

- [ ] 改断言（现在断言的是自动切换）：
  - `routing/mod.rs`：`all_filtered_candidates_fallback_to_parent`、`llm_lower_weights_are_tried_only_when_higher_groups_are_unavailable`（改为只看静态）、`builtin_fallback_routes_tier_first_then_by_the_requested_scheduler`、`exact_route_fallback_requires_explicit_policy_and_target`。
  - `execution/mod.rs`：`unavailable_model_skips_same_model_retry_and_fails_over`、`decision_failover_preserves_api_and_writes_only_successful_usage` 及其它 `runtime_failover = true` 的用例。
  - `service/tests.rs`：`builtin_single_llm_serves_every_usecase_it_is_capable_of`、`builtin_section_2_1_models_reach_every_usecase_alone`、`builtin_fallback_*`。
  - `health.rs`：改为只影响展示与告警。
- [ ] 新增：两个 Provider 同池，第一个连续失败 10 次，第二个收到 0 次请求；404 之后立刻再请求仍选同一模型；overlay 不能打开或改 fallback 目标；管理员配置的 fallback 只执行一次。
- [ ] T1 验收：`test/aicc_test/acceptance/run_t1_gateway.ts`、`logical_entrypoint_baseline.json`、`doc/aicc/maintenance/acceptance/cases/` 下断言 failover 的用例（尤其 `cases_l1_routing_scheduler.md`、`cases_l3_local_p0_core.md`）。
- [ ] 手册：`src/apps/jarvis_agent/agent/cmd_skills/xllm/references/runs.md`（写着 `llm.fallback` 会选最便宜的合格档）、`buckyos_admin/references/providers.md`（补 fallback 配置与故障排查）。
- [ ] 真实 Zone：停掉选中的 Provider，确认报真实错误、没有换模型、管理员收到通知；配置 fallback 后再试一次，核对用量归因。

## 4. 与 PR #751 的关系

PR #751 想在 404 后自动重排一次路由，与原则 3 冲突，这部分不合入。它的凭证重登、`no_candidate_model` / `resource_not_permitted` 拆分可以保留，正好服务 T1、T2。它按图片参数筛选候选的做法见 Q6。

## 5. 待确认

括号里是建议；没有答复前不实施对应部分。

- **Q1 触发条件。** 显式 fallback 在哪些失败时触发？（建议：无静态候选、Provider 重试后仍不可用、模型不存在、上游配额耗尽、用户额度拒绝。400 / 401 / 403 / 409、取消、已产生用量的失败不触发。）
- **Q2 覆盖率缺口。** 去掉 `llm.fallback` 后，只接了一个小模型的 Zone 里 `llm.plan`、`llm.code` 可能没有候选。（建议：不恢复内置兜底，改为接入 Provider 时在 AI Center 列出缺口，由用户一键确认写入路由配置。）
- **Q3 内置用途权重。** 用途目录里 20 多个厂商规格按权重排列，T2 之后只表示“接入了哪些模型时默认用谁”。（建议：保留。）
- **Q4 默认选择随库存和 metadata 变化。** Provider 上新、metadata 云端更新都会改变默认选中的模型，可能换成更贵的。（建议：变化先提示管理员，确认后生效；与 [metadata 云端更新 TODO](aicc-driver-metadata-cloud-update-todo.md) 一起定。）
- **Q5 是否整个移除 `parent` 模式与 `allow_exact_model_fallback`。**（建议：移除，它们都会选到管理员没有点名的模型，或让请求方自己换模型。）
- **Q6 请求参数能否改变选中的模型。** 例如图片请求多带一个 `aspect_ratio` 就换到别家。（建议：可选的调节参数不参与选模型；先按配置选定，不支持就返回 `unsupported_operation`，写明模型和字段。tool call、视觉、上下文长度等“没有就做不了”的能力仍是硬过滤。）
- **Q7 同权重池内的选择。** 现在按 `cost_first` 评分，依赖动态成本估算，可能在两家之间来回。（建议：价格未知时按配置顺序固定选第一个，评分只用静态输入。）

## 6. 已修订的文档

2026-11-23 已改：`doc/aicc/` 下的 `aicc_router.md`（§9 重写）、`aicc 逻辑模型目录.md`、`aicc_api设计.md`、`AICC.md`、`aicc-models-mgr.md`、`aicc_token_quota.md`、`aicc_agent_cli_tools.md`、`aicc_e2e_test_requirements.md` §7.7、`driver_metadata_schema.md`、`video_generation.md`、`frozen_user_api.md`、`frozen_model_driver_and_logical_model_fs.md`；`maintenance/` 下的 `llm_catalog_routine_maintenance.md`、`krpc_aicc_calling_guide.md`、`aicc_maintenance_roles.md`、`README.md`；`notepads/` 下的 `aicc-zone-aicc-provider-todo.md`、`aicc-model-catalog-maintenance-todo.md`。

没有改，随代码一起改：验收用例（`maintenance/acceptance/cases/`）、Agent 手册、`model_defaults.rs` 注释。没有改，属历史记录：`archive/`、`notepads/done/`、`driver_metadata_review_*.md`、`aicc_reimplementation_roadmap.md`。
