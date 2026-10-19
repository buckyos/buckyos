# AICC E2E Acceptance Report

## 人工复核与清理补充（2026-10-19）

本轮选中 42 个 SN 模型/API 单元，另有 10 次前置生成，共 52 次推理 RPC，均形成 AICC 任务；没有 `ExpiredSignature`。原始结果为 8 passed、14 failed、20 review，usage/trace 审计通过。

20 个 review 单元人工复核为 16 passed、1 failed、3 review；计入原有 8 passed 后，本轮有效结果为 **24 passed、15 failed、3 review**。下方 runner 原始状态不改写。

| 模型 | API | 人工结论 | 内容证据 |
|---|---|---|---|
| gpt-5.6-luna | image.img2img | review | 结构、制品读取与审计通过；未留存可供语义复核的输出副本 |
| gpt-5.6-sol | image.txt2img | passed | 蓝色方块内清晰显示 4827 |
| gpt-5.6-sol | vision.caption | passed | 正确描述粉花、黄色中心、绿叶和椭圆背景 |
| gpt-5.6-sol | vision.ocr | passed | `BJCKY06-0y-4827` 含 rubric 要求的数字；不表示全文无误 |
| gpt-5.6-terra | image.txt2img | passed | 蓝色正方形画布内清晰显示 4827 |
| gpt-5.6-terra | vision.caption | passed | 正确描述花朵、茎叶及背景 |
| gpt-5.6-terra | vision.ocr | failed | `RUCKTOS CV-482?` 未完整读出 4827 |
| gpt-6-astra | image.img2img | review | 结构、制品读取与审计通过；尚缺语义复核 |
| gpt-6-astra | vision.caption | passed | 正确描述粉色花朵、黄心、绿叶和五个背景椭圆 |
| gpt-6-astra | vision.ocr | passed | 完整识别 `BUCKYOS-DV-4827` |
| gpt-6-luna | vision.caption | passed | 正确描述花朵、茎叶、椭圆和水平地面 |
| gpt-6-luna | vision.ocr | passed | `BINARY 0V-4827` 含 4827；其余字符不完全准确 |
| gpt-6-sol | image.txt2img | passed | 蓝色方块内清晰显示 4827 |
| gpt-6-sol | vision.caption | passed | 正确描述粉花、黄色中心、绿叶和椭圆背景 |
| gpt-6-sol | vision.ocr | passed | `BICKYNS-04-4827` 含 4827；其余字符不完全准确 |
| gpt-image-2 | image.img2img | review | 结构、制品读取与审计通过；尚缺语义复核 |
| gpt-image-2 | image.inpaint | passed | 蓝色方形画布中心生成合理绿色叶簇，符合本轮 mask guidance rubric |
| gpt-image-2 | image.txt2img | passed | 蓝色方形画布内清晰显示 4827 |
| kimi-k3 | vision.caption | passed | 正确描述花朵、茎叶、背景分界及五个椭圆 |
| kimi-k3 | vision.ocr | passed | `BUCKY05-0V-4827` 含 4827；其余字符不完全准确 |

14 个自动失败包括 9 个 HTTP 500 和 5 个 computer-use。旧 computer-use 请求要求报告标题，却只提供 1×1 图片并断言固定点击，属于无效语义用例；其有效验收以随后按 [官方 computer-use 流程](https://developers.openai.com/api/docs/guides/tools-computer-use) 构建的截图点击用例为准，不由本轮错误反推模型不支持 computer-use。

runner 移除 18 个输出对象后，另核对并取消本轮 9 个非终态任务，全部进入 `Terminal/Canceled`。原任务与 usage/trace 留作审计。SN 实际 USD 费用未知，`$0.52` 仅为配置的每调用 `$0.01` 估算，不是账单或实付费用上限。

- Run: `aicc-2026-10-19T07-48-16-711Z-bac3f0f4`
- Commit: `2dd069965d82127790c093dc5fc04ec331b9efa9`
- Capability baseline: `2026-10-19.3`
- Real model calls: 52/52
- Planned maximum cost: $0.520000
- Known actual / unknown estimated exposure: $0.000000 / $0.520000
- Total exposure / budget: $0.520000 / $0.580000
- Unknown-cost calls: 52; budget exceeded: false
- Results: passed=8, failed=14, provider_restricted=0, skipped=0, not_applicable=0, review=20

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider sn-ai-provider --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-mini.image.img2img --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-nano.image.img2img --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.5.image.img2img --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.agent.computer_use --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.image.img2img --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.agent.computer_use --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.image.img2img --case t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.agent.computer_use --case t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.image.txt2img --case t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.agent.computer_use --case t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.image.img2img --case t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.image.txt2img --case t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.agent.computer_use --case t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.image.img2img
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-mini.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4-mini@sn-ai-provider-default | image.img2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-nano.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4-nano@sn-ai-provider-default | image.img2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.5.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.5@sn-ai-provider-default | image.img2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-luna.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-luna@sn-ai-provider-default | image.img2img | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-sol@sn-ai-provider-default | agent.computer_use | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-sol@sn-ai-provider-default | image.img2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-sol@sn-ai-provider-default | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.llm | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-sol@sn-ai-provider-default | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.vision.caption | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-sol@sn-ai-provider-default | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-sol@sn-ai-provider-default | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra@sn-ai-provider-default | agent.computer_use | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra@sn-ai-provider-default | image.img2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra@sn-ai-provider-default | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.llm | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra@sn-ai-provider-default | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.vision.caption | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra@sn-ai-provider-default | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra@sn-ai-provider-default | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-astra@sn-ai-provider-default | agent.computer_use | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-astra@sn-ai-provider-default | image.img2img | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-astra@sn-ai-provider-default | image.txt2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.llm | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-astra@sn-ai-provider-default | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.vision.caption | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-astra@sn-ai-provider-default | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-astra@sn-ai-provider-default | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-luna@sn-ai-provider-default | agent.computer_use | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-luna@sn-ai-provider-default | image.img2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-luna@sn-ai-provider-default | image.txt2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.llm | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-luna@sn-ai-provider-default | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.vision.caption | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-luna@sn-ai-provider-default | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-luna@sn-ai-provider-default | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-sol@sn-ai-provider-default | agent.computer_use | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-sol@sn-ai-provider-default | image.img2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-sol@sn-ai-provider-default | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.llm | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-sol@sn-ai-provider-default | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.vision.caption | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-sol@sn-ai-provider-default | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-sol@sn-ai-provider-default | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-image-2.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-image-2@sn-ai-provider-default | image.img2img | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-image-2.image.inpaint | T2 | sn-ai-provider/sn-ai-provider-default | gpt-image-2@sn-ai-provider-default | image.inpaint | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-image-2.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-image-2@sn-ai-provider-default | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-default.kimi-k3.llm | T2 | sn-ai-provider/sn-ai-provider-default | kimi-k3@sn-ai-provider-default | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-default.kimi-k3.vision.caption | T2 | sn-ai-provider/sn-ai-provider-default | kimi-k3@sn-ai-provider-default | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-default.kimi-k3.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | kimi-k3@sn-ai-provider-default | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-default.text-embedding-3-large.embedding.text | T2 | sn-ai-provider/sn-ai-provider-default | text-embedding-3-large@sn-ai-provider-default | embedding.text | passed |
| t2.sn-ai-provider.sn-ai-provider-default.text-embedding-3-small.embedding.text | T2 | sn-ai-provider/sn-ai-provider-default | text-embedding-3-small@sn-ai-provider-default | embedding.text | passed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| sn-ai-provider/sn-ai-provider-default | gpt-5.4-mini | gpt-5.4-mini | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.4-nano | gpt-5.4-nano | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.5 | gpt-5.5 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-luna | gpt-5.6-luna | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-sol | gpt-5.6-sol | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra | gpt-5.6-terra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-astra | gpt-6-astra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-luna | gpt-6-luna | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-sol | gpt-6-sol | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-image-2 | gpt-image-2 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | kimi-k3 | kimi-k3 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | text-embedding-3-large | text-embedding-3-large | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | text-embedding-3-small | text-embedding-3-small | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.4-mini | gpt-5.4-mini | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.4-nano | gpt-5.4-nano | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.5 | gpt-5.5 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-luna | gpt-5.6-luna | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-sol | gpt-5.6-sol | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra | gpt-5.6-terra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-astra | gpt-6-astra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-luna | gpt-6-luna | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-sol | gpt-6-sol | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-image-2 | gpt-image-2 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | kimi-k3 | kimi-k3 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | text-embedding-3-large | text-embedding-3-large | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | text-embedding-3-small | text-embedding-3-small | included | physical model | - |

## Confirmed product defects

- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-mini.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-mini.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-nano.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-nano.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.5.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.5.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.image.img2img.json

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 18 generated output object(s)
