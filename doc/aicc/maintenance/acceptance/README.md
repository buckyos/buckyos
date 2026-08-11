# AICC 验收用例拆分索引

本目录把 AICC 验收方案拆分为可逐步实现和评审的自包含任务文档。每个拆分文档都携带本任务需要的详细矩阵、字段、case id、命令或判定规则。

## 阅读规则

- 做某个任务时，先读该任务文档，再读依赖图中它上游的基础文档。
- 基础文档定义全局约束；组件文档定义 runner / mock / report / gateway 等可复用能力；cases 文档定义具体用例组。
- 如果同一细节在多个文档出现，以更具体的 cases 或 components 文档为准。

## 依赖图

```text
00_acceptance_principles.md
  -> 01_acceptance_architecture.md
  -> 02_case_manifest_and_report.md
  -> 03_environment_safety_and_cleanup.md
  -> 04_coverage_and_trace_matrix.md
  -> 05_method_acceptance_matrix.md
      -> components/component_fixtures.md
      -> components/component_report_runner.md
      -> components/component_redaction_and_diagnostics.md
      -> components/component_mock_provider_contract.md
          -> components/component_mock_provider_part_1.md
          -> components/component_mock_provider_part_2.md
          -> components/component_l3_local_runner.md
              -> cases/cases_l3_local_p0_core.md
              -> cases/cases_l3_config_reload_admin.md
      -> components/component_provider_protocol_coverage.md
      -> components/component_l4_gateway_environment.md
          -> components/component_l4_matrix_runner.md
              -> cases/cases_l4_gateway_llm_providers.md
              -> cases/cases_l4_gateway_media_providers.md
      -> components/component_ci_release_gate.md
      -> cases/cases_l1_routing_scheduler.md
      -> cases/cases_l1_provider_protocol_openai.md
      -> cases/cases_l1_resources_tasks_usage_security.md
      -> cases/cases_l2_aicc_client.md
      -> cases/cases_p0_minimum_set.md
      -> cases/cases_exception_paths.md
      -> cases/cases_maintenance_update.md
```

## 基础文档

| 文档 | 内容 |
|---|---|
| `00_acceptance_principles.md` | 目标、设计依据、协议约束、优先级、确定性、风险、第一阶段边界 |
| `01_acceptance_architecture.md` | L1/L2/L3/L4 架构、统一执行脚本、CI/手工边界、里程碑、命令、任务拆分 |
| `02_case_manifest_and_report.md` | 报告 schema、case id、manifest、failure_class、诊断字段、报告示例 |
| `03_environment_safety_and_cleanup.md` | preflight、cleanup、真实模型开关、临时 group 清理 |
| `04_coverage_and_trace_matrix.md` | 功能覆盖矩阵、需求追踪矩阵 |
| `05_method_acceptance_matrix.md` | 所有 method 输入/输出/异常矩阵、真实模型判定规则 |

## 组件文档

| 文档 | 内容 |
|---|---|
| `components/component_fixtures.md` | 测试资源、fixture manifest、digest/media type 校验 |
| `components/component_provider_protocol_coverage.md` | Provider 协议覆盖和 L4 真实 Provider 矩阵 |
| `components/component_mock_provider_contract.md` | Mock Provider 行为、配置样例、HTTP 接口契约 |
| `components/component_mock_provider_part_1.md` | TS Mock Provider 管理接口和 OpenAI-like 路径 |
| `components/component_mock_provider_part_2.md` | Claude-like、Gemini-like、fal-like Mock 协议扩展 |
| `components/component_l3_local_runner.md` | L3 本地 kRPC runner、reload、models.list、清理 |
| `components/component_l4_gateway_environment.md` | Gateway 环境、TOML、buckyos-devkit 临时 group |
| `components/component_l4_matrix_runner.md` | L4 五维矩阵、attempt、partial/skipped/not_applicable |
| `components/component_redaction_and_diagnostics.md` | 脱敏、安全扫描、失败诊断字段 |
| `components/component_report_runner.md` | summary.json / summary.md、attempt、报告示例 |
| `components/component_ci_release_gate.md` | CI、发布门槛、性能并发、文档联动、评审清单 |

## 用例组文档

| 文档 | 内容 |
|---|---|
| `cases/cases_l1_routing_scheduler.md` | L1 路由、fallback、调度、overlay、硬过滤、trace |
| `cases/cases_l1_provider_protocol_openai.md` | L1 Provider adapter 协议转换 |
| `cases/cases_l1_resources_tasks_usage_security.md` | L1 ResourceRef、artifact、task、usage、安全 |
| `cases/cases_l2_aicc_client.md` | L2 AiccClient 黑盒测试 |
| `cases/cases_l3_local_p0_core.md` | L3 本地 P0 闭环 |
| `cases/cases_l3_config_reload_admin.md` | L3 reload、models.list、provider 管理、usage/quota |
| `cases/cases_l4_gateway_llm_providers.md` | L4 真实 LLM Provider gateway workflow |
| `cases/cases_l4_gateway_media_providers.md` | L4 真实媒体 Provider gateway workflow |
| `cases/cases_maintenance_update.md` | 模型/Provider/metadata/routing/策略更新和回滚 |
| `cases/cases_p0_minimum_set.md` | M0/M1 首批 P0 最小 case id |
| `cases/cases_exception_paths.md` | 异常路径矩阵 |
