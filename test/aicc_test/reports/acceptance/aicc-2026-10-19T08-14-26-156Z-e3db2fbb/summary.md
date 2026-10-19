# AICC E2E Acceptance Report

## 有效执行边界

本轮使用新的 1280×720 蓝色按钮截图，要求在按钮范围内返回一次左键动作，依据 [OpenAI computer-use 官方流程](https://developers.openai.com/api/docs/guides/tools-computer-use)。不在真实桌面执行点击。

6 次推理 RPC 中只有 2 次形成 AICC 任务：`gpt-5.6-luna` 返回 `OpenAI response has no computer action`；`gpt-5.6-sol` 返回 `Provider returned a computer action that the caller did not allow`。其余 4 次在路由阶段被 `circuit_open` 拒绝，没有到达 Provider，须以随后补齐的有效调用结果为准；不能当成 4 个新的 Provider 推理失败。

下方自动计数 `6` 为 RPC 尝试数，实际接受任务 2 项；配置估算敞口相应为 `$0.02`，实际费用未知。cleanup 通过。

- Run: `aicc-2026-10-19T08-14-26-156Z-e3db2fbb`
- Commit: `2dd069965d82127790c093dc5fc04ec331b9efa9`
- Capability baseline: `2026-10-19.3`
- Real model calls: 6/6
- Planned maximum cost: $0.060000
- Known actual / unknown estimated exposure: $0.000000 / $0.060000
- Total exposure / budget: $0.060000 / $0.060000
- Unknown-cost calls: 6; budget exceeded: false
- Results: passed=0, failed=6, provider_restricted=0, skipped=0, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider sn-ai-provider --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-luna.agent.computer_use --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.agent.computer_use --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.agent.computer_use --case t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.agent.computer_use --case t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.agent.computer_use --case t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.agent.computer_use
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-luna.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-luna@sn-ai-provider-default | agent.computer_use | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-sol@sn-ai-provider-default | agent.computer_use | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra@sn-ai-provider-default | agent.computer_use | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-astra@sn-ai-provider-default | agent.computer_use | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-luna@sn-ai-provider-default | agent.computer_use | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-sol@sn-ai-provider-default | agent.computer_use | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-luna | gpt-5.6-luna | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-sol | gpt-5.6-sol | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra | gpt-5.6-terra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-astra | gpt-6-astra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-luna | gpt-6-luna | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-sol | gpt-6-sol | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-luna | gpt-5.6-luna | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-sol | gpt-5.6-sol | included | physical model | - |
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
