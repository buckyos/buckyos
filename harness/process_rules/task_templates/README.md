# 系统服务任务模版索引

本目录中的模版用于配合 [Service Dev Loop](../../Service%20Dev%20Loop.md) 和 [plan_proposal.md](../plan_proposal.md) 进行任务拆分。

使用原则：

- 一个 task 只对应一个负责人。
- 关注流程中的 checkpoint，至少保证每两个 checkpoint 之间有一个 task。
- 优先选择能直接套用的模版，减少临时自由发挥。
- `task_template.md` 仍然保留为通用兜底模版；本目录新增的模版用于系统服务主线开发。

推荐模版与适用阶段：

- `system_service_protocol_design_task_template.md`
  - 用于协议设计阶段，产出 KRPC 协议文档并冻结服务边界。
- `system_service_durable_data_schema_task_template.md`
  - 用于持久数据格式设计阶段，明确 durable/disposable 数据与兼容策略。
- `system_service_implementation_and_cargo_test_task_template.md`
  - 用于实现主体与单元测试阶段，目标是通过 `cargo test`。
- `system_service_runtime_integration_task_template.md`
  - 用于接入 `buckyos build`、scheduler、Service Spec、Settings Schema、login/heartbeat 的运行接入阶段。
- `system_service_dv_test_task_template.md`
  - 用于 TypeScript DV Test 阶段，验证真实 gateway 路径和 SDK 可用性。
- `system_service_developer_loop_task_template.md`
  - 用于本地 Developer Loop 收敛阶段，明确测试、日志、重建和重部署闭环。
- `system_service_sdk_and_simple_integration_task_template.md`
  - 用于 SDK 正式提交与安装包环境 Simple Integration Test 收尾阶段。

建议拆分顺序：

1. 协议设计
2. 持久数据格式设计
3. 实现与 `cargo test`
4. 运行接入
5. DV Test
6. Developer Loop
7. SDK / Simple Integration Test

## WebUI 任务模版

本目录也包含配合 [WebUI Dev Loop](../../WebUI%20Dev%20Loop.md) 使用的 WebUI 任务模版，适用于“为已有系统服务添加 WebUI”的任务拆分。

推荐模版与适用阶段：

- `webui_prd_and_design_input_task_template.md`
  - 用于 PRD 审核、UI 设计提示整理和启动条件确认。
- `webui_initial_datamodel_task_template.md`
  - 用于初版 UI DataModel 定义与文档冻结前的第一版建模。
- `webui_mock_first_prototype_task_template.md`
  - 用于 Mock-first Prototype、`pnpm run dev` 独立运行、Mock Data 与 Playwright 自动化。
- `webui_pr_review_and_experience_convergence_task_template.md`
  - 用于 UI PR 阶段的产品体验收敛和 DataModel 冻结。
- `webui_backend_integration_and_benchmark_task_template.md`
  - 用于 DataModel × Backend 集成、性能测试与前后端模型收敛。
- `webui_dv_test_task_template.md`
  - 用于真实系统链路下的 UI DV Test。

建议拆分顺序：

1. PRD / UI 设计输入
2. 初版 UI DataModel
3. Mock-first Prototype
4. UI PR / 体验收敛 / DataModel 冻结
5. DataModel × Backend 集成 / 性能测试
6. UI DV Test
