# AICC E2E Acceptance Report

## 有效执行边界

4 次推理 RPC 中只有 2 次形成 AICC 任务：`gpt-5.6-terra` 返回不在允许列表的 computer action；`gpt-6-luna` 返回 `OpenAI response has no computer action`。`gpt-6-astra` 和 `gpt-6-sol` 在路由阶段被 `circuit_open` 拒绝，没有到达 Provider，须以后续有效调用为准。

本轮 31 秒最小间隔以请求开始时间计算，并非从失败完成时间起算，因此未完全覆盖 30 秒冷却。该事实不改变 computer-use 的动作断言。下方自动计数 `4` 为 RPC 尝试，实际接受任务 2 项；配置估算敞口 `$0.02`，实际费用未知。cleanup 通过。

- Run: `aicc-2026-10-19T08-15-30-112Z-b647e6ac`
- Commit: `2dd069965d82127790c093dc5fc04ec331b9efa9`
- Capability baseline: `2026-10-19.3`
- Real model calls: 4/4
- Planned maximum cost: $0.040000
- Known actual / unknown estimated exposure: $0.000000 / $0.040000
- Total exposure / budget: $0.040000 / $0.040000
- Unknown-cost calls: 4; budget exceeded: false
- Results: passed=0, failed=4, provider_restricted=0, skipped=0, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider sn-ai-provider --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.agent.computer_use --case t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.agent.computer_use --case t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.agent.computer_use --case t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.agent.computer_use
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra@sn-ai-provider-default | agent.computer_use | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-astra@sn-ai-provider-default | agent.computer_use | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-luna@sn-ai-provider-default | agent.computer_use | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-sol@sn-ai-provider-default | agent.computer_use | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra | gpt-5.6-terra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-astra | gpt-6-astra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-luna | gpt-6-luna | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-sol | gpt-6-sol | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra | gpt-5.6-terra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-astra | gpt-6-astra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-luna | gpt-6-luna | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-sol | gpt-6-sol | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
