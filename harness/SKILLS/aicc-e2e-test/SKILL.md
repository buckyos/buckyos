---
name: aicc-e2e-test
description: 实现、审查或执行 BuckyOS AICC T1/T1.5/T2/T3 E2E 验收测试，尤其适用于涉及 Provider 协议 Mock、真实 Provider、消息入口、凭证、成本或临时 AICC 设置的任务。
---

# AICC E2E 测试

以 `doc/aicc/aicc_e2e_test_requirements.md` 作为规范性测试要求，以 `test/aicc_test/acceptance/README.md` 作为测试运行命令说明。

## CodeAgent 执行授权

CodeAgent 开始执行任何 T2 或 T3 测试前，必须取得用户对本次执行的明确授权。已提交到仓库的配置、已启用的 `allow_real_model_calls`、机器上已有的凭证、此前的测试执行或先前对话轮次中的批准，都不构成对新一次执行的授权。

当用户当前请求明确要求执行相应范围的 T2/T3 时，即视为授权充分。否则必须在执行 runner 命令前停止，并说明：

- 选定的测试层及 case ID 或场景；
- 可能使用的 Provider driver 和消息入口；
- 真实模型调用次数上限及成本预算；
- 是否会临时更改 Provider 凭证或 AICC 设置；
- 是否会创建外部消息或制品。

不得将一次授权扩展到额外的 Provider、入口、用例、重试、配置变更或更高预算。当已授权范围发生实质变化，或需要在之后再次执行时，必须重新请求授权。

静态 preflight、单元测试/自测、报告检查，以及既不连接也不修改 DV 环境的 dry-run，无需此项授权。T1/T1.5 Mock 执行遵循其自身明确的配置变更保护机制，并且不得调用真实 Provider。

runner 的 `--yes` 选项仅用于已经获得授权的自动化；它本身不授予 CodeAgent 执行 T2/T3 的权限。本 Skill 约束的是 CodeAgent 的行为，不约束 CI 或人工操作者直接运行命令。

## 执行不变量

- 使用 Zone Gateway 和真实鉴权路径；不得直接调用服务端口。
- 每个 T1.5 Provider Mock 及其预期 wire fixture，只能依据该 Provider 的官方 API 文档、官方 schema、官方 SDK 协议定义和官方错误文档构建。不得根据 AICC 设计文档、metadata、实现代码、现有请求日志或现有 Mock 推导预期协议行为。
- T2 必须限制在用户选定的 Provider 和 instance 范围内。T3 必须限制在已获授权的 Provider 和消息入口范围内；配置的 T3 instance 用于凭证注入和审计，不用于强制路由。
- T1.5 必须将每个可独立调用的 metadata variant 作为独立协议 cell 覆盖，包括 Provider 模型/选项 lowering 和响应解析。
- T2 覆盖范围限制为 `ProviderInstance x model x API-Type` 矩阵。必须包含范围内每个活跃的基础物理模型，但不得为 metadata variant 增加真实推理 cell。除非明确的 rubric 要求最小限度的补充，否则每个 cell 只运行一个最小正确性用例。T2 不得重复同一物理模型的逻辑别名、T1 路由组合或 T1.5 wire/error 覆盖。
- 真实调用前，打印并强制执行调用次数、重试、超时和预算限制。T1/T1.5/T2 还必须强制执行全局及每个 Provider 的并发限制和最小请求间隔；T3 只强制执行场景并发限制。
- 机密信息只能保存在本地且被忽略的 TOML 文件中，并且必须从命令、日志和报告中脱敏。
- 恢复临时设置和资源，然后报告清理结果、实际调用次数、成本、失败情况及定向重测命令。
- 当确认某个失败属于 AICC/Jarvis 缺陷时，应保持测试断言正确，并记录预期行为、实际行为和证据；没有单独请求时不得修改产品代码。

## 开发与推送节奏

- 每轮 coding 期间，只运行与本轮所修改实现相对应的单元测试。不得反复 build、deploy 或运行 T1/T1.5/T2/T3，以此替代内循环中的针对性单元测试。
- 每次 Git push 前，如果待推送变更更新了路由逻辑，运行受影响的 T1 测试套件。
- 每次 Git push 前，如果待推送变更更新了 Provider API 协议实现、模型 metadata、Provider metadata 或 Provider 配置，只对受影响的一个或多个 Provider 运行 T1.5。
- 如果一次 push 同时包含路由和 Provider 协议/metadata/配置变更，运行上述两项必要门禁。仅含文档或无关改动的 push 不会因这些规则而产生 E2E 执行义务。

## E2E 自动化收敛循环

当 coding 目标明确为 T1/T1.5/T2/T3 自动化时，应按批次工作：

1. 在编辑实现前，运行选定范围并收集一批带证据的失败。
2. 对该批失败进行分类，集中修复其中的实现问题，然后针对这一批次只 build 和 deploy 一次。
3. 集中测试受影响的这一批用例并记录结果。
4. 如果本轮修改了实现，在本轮结束后重新运行相同的选定范围。
5. 持续执行，直到一次完整重跑通过，并且从前一次运行到该次通过之间没有再修改实现。

不得默认采用 `一个失败 -> 修复一个问题 -> build -> deploy -> test -> accept` 的循环。如果最终重跑仅剩少量已确认并非由实现逻辑引起的失败，例如偶发网络不稳定、鉴权失败或账户余额不足，可以终止执行。最终报告必须列出每个此类失败、受影响的 case 和 Provider、支持证据，以及无需再次修改实现或重跑的原因。
