# AICC E2E Acceptance Report

- Run: `aicc-2026-10-19T09-30-39-700Z-e72f3108`
- Commit: `2dd069965d82127790c093dc5fc04ec331b9efa9`
- Capability baseline: `2026-10-19.3`
- Real model calls: 7/13
- Planned maximum cost: $0.130000
- Known actual / unknown estimated exposure: $0.000000 / $0.130000
- Total exposure / budget: $0.130000 / $0.300000
- Unknown-cost calls: 7; budget exceeded: false
- Results: passed=6, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=1

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-nano.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4-nano@sn-ai-provider-default | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-luna.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-luna@sn-ai-provider-default | agent.computer_use | passed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-sol@sn-ai-provider-default | agent.computer_use | passed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra@sn-ai-provider-default | agent.computer_use | passed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-astra@sn-ai-provider-default | agent.computer_use | passed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-luna@sn-ai-provider-default | agent.computer_use | passed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-sol@sn-ai-provider-default | agent.computer_use | passed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| sn-ai-provider/sn-ai-provider-default | gpt-5.4-nano | gpt-5.4-nano | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-luna | gpt-5.6-luna | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-sol | gpt-5.6-sol | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra | gpt-5.6-terra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-astra | gpt-6-astra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-luna | gpt-6-luna | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-sol | gpt-6-sol | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.4-nano | gpt-5.4-nano | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-luna | gpt-5.6-luna | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-sol | gpt-5.6-sol | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra | gpt-5.6-terra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-astra | gpt-6-astra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-luna | gpt-6-luna | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-sol | gpt-6-sol | included | physical model | - |

## Confirmed product defects

None.

## 人工复核与任务审计

- 最终验收：6 passed、1 failed、0 review。原始机器结果保持不变。
- `gpt-5.4-nano/vision.ocr`：任务 `t-00e939ba0d8d45f38f668b6ffb66c691`，TaskMgr 为 Terminal/Succeeded；实际文本为 `E ICKY’O’S O II’I’TE OI?H E?P`，未识别输入中的 `BUCKYOS-DV-4827`，内容验收 failed。异步提交、查询和结果读取成功，不属于异步接口缺口。
- 六个 computer-use 任务全部 Terminal/Succeeded，均返回允许的 `left_click(640,360)`，位于测试按钮内；每个任务的 `usage.request_units=2`，包含首次截图请求及关联截图回传。
- 本轮接受 7 个 AICC 任务、13 次上游推理请求（OCR 1 次、computer-use 12 次）；轮询 GET 不计为推理。13 次的未知费用估算为 $0.13，非实际账单。临时配置已恢复，7 个任务均已终止，无遗留制品。

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
