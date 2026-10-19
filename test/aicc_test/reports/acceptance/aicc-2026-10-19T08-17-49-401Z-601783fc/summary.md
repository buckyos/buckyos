# AICC E2E Acceptance Report

## 最终执行与清理核验（2026-10-19）

两项均实际形成 AICC 任务：`gpt-6-astra` 返回 `OpenAI response has no computer action`；`gpt-6-sol` 返回 `Provider returned a computer action that the caller did not allow`。本轮无认证过期或熔断拦截；至此 6 个 SN computer-use 单元均已有有效后端调用结果，全部 failed，不能由旧 1×1 fixture 或路由拒绝结果替代这些结论。

五轮按 run ID 只读回查确认：100 个 AICC 任务均已终止，74 Succeeded、12 Failed、14 Canceled；这包括前置生成及旧用例，不等于模型/API 通过数。28 个输出对象逐一回读均返回不存在，无可读残留。临时 `sn-ai-provider-default` 已移除，既有配置保留，运行时 SN 测试实例不存在。

模型/API 的去重合并及人工复核结论为 80 个单元中 53 passed、23 failed、4 review，见 [SN 汇总](../../../../../sn_REPORT.md)。其中 79 个实际发出有效目标请求，另 1 个因前置生成失败而 failed。五轮共 148 次 RPC 尝试，42 次认证拒绝和 6 次熔断拒绝不属于 Provider 推理；实际接受任务 100 项。按本地每项 `$0.01` 估算为 `$1.00`，实际账单未知，不是实际扣费上限。

- Run: `aicc-2026-10-19T08-17-49-401Z-601783fc`
- Commit: `2dd069965d82127790c093dc5fc04ec331b9efa9`
- Capability baseline: `2026-10-19.3`
- Real model calls: 2/2
- Planned maximum cost: $0.020000
- Known actual / unknown estimated exposure: $0.000000 / $0.020000
- Total exposure / budget: $0.020000 / $0.020000
- Unknown-cost calls: 2; budget exceeded: false
- Results: passed=0, failed=2, provider_restricted=0, skipped=0, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider sn-ai-provider --case t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.agent.computer_use --case t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.agent.computer_use
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-astra@sn-ai-provider-default | agent.computer_use | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-sol@sn-ai-provider-default | agent.computer_use | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| sn-ai-provider/sn-ai-provider-default | gpt-6-astra | gpt-6-astra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-sol | gpt-6-sol | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-astra | gpt-6-astra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-sol | gpt-6-sol | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
