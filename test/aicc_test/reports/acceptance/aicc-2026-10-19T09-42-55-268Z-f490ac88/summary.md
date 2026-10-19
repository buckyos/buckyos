# AICC E2E Acceptance Report

- Run: `aicc-2026-10-19T09-42-55-268Z-f490ac88`
- Commit: `2dd069965d82127790c093dc5fc04ec331b9efa9`
- Capability baseline: `2026-10-19.3`
- Real model calls: 24/24
- Planned maximum cost: $0.240000
- Known actual / unknown estimated exposure: $0.000000 / $0.240000
- Total exposure / budget: $0.240000 / $0.700000
- Unknown-cost calls: 24; budget exceeded: false
- Results: passed=0, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=16

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-mini.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4-mini@sn-ai-provider-default | image.img2img | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-nano.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4-nano@sn-ai-provider-default | image.img2img | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-nano.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4-nano@sn-ai-provider-default | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-pro.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4-pro@sn-ai-provider-default | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-pro.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4-pro@sn-ai-provider-default | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4@sn-ai-provider-default | image.img2img | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4@sn-ai-provider-default | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.5-pro.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.5-pro@sn-ai-provider-default | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.5.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.5@sn-ai-provider-default | image.img2img | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-sol@sn-ai-provider-default | image.img2img | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra@sn-ai-provider-default | image.img2img | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra@sn-ai-provider-default | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-astra@sn-ai-provider-default | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-luna@sn-ai-provider-default | image.img2img | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-luna@sn-ai-provider-default | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-sol@sn-ai-provider-default | image.img2img | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| sn-ai-provider/sn-ai-provider-default | gpt-5.4-mini | gpt-5.4-mini | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.4-nano | gpt-5.4-nano | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.4-pro | gpt-5.4-pro | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.4 | gpt-5.4 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.5-pro | gpt-5.5-pro | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.5 | gpt-5.5 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-sol | gpt-5.6-sol | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra | gpt-5.6-terra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-astra | gpt-6-astra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-luna | gpt-6-luna | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-sol | gpt-6-sol | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.4-mini | gpt-5.4-mini | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.4-nano | gpt-5.4-nano | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.4-pro | gpt-5.4-pro | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.4 | gpt-5.4 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.5-pro | gpt-5.5-pro | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.5 | gpt-5.5 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-sol | gpt-5.6-sol | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra | gpt-5.6-terra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-astra | gpt-6-astra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-luna | gpt-6-luna | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-sol | gpt-6-sol | included | physical model | - |

## Confirmed product defects

None.

## 人工复核与任务审计

最终验收为 **16 passed、0 failed、0 review、0 skipped**。原始机器结果 `review=16` 保持不变；以下为已下载制品及实际 OCR 文本的人工复核，未新增推理或 Judge 调用。

| 模型 | API | 最终结果 | 复核依据 |
|---|---|---|---|
| gpt-5.4-pro | image.txt2img | passed | 蓝色方块内清晰可见 4827 |
| gpt-5.4 | image.txt2img | passed | 蓝色方块内清晰可见 4827 |
| gpt-5.4-nano | image.txt2img | passed | 蓝色画布中清晰可见 4827 |
| gpt-6-astra | image.txt2img | passed | 蓝色方块内清晰可见 4827 |
| gpt-6-luna | image.txt2img | passed | 蓝色方块内清晰可见 4827 |
| gpt-5.4-pro | vision.ocr | passed | 实际文本含 PICKYOS-OV-4827，满足原定数字 4827 rubric；不代表整行逐字正确 |
| gpt-5.5-pro | vision.ocr | passed | 实际文本 BUCKY05-DV-4827，满足原定数字 4827 rubric |
| gpt-5.6-terra | vision.ocr | passed | 实际文本 BICKTOS CV-4827，满足原定数字 4827 rubric；不代表整行逐字正确 |
| gpt-5.4 | image.img2img | passed | 对照源图保留方块与数字中心构图，应用暖金色光照 |
| gpt-5.4-mini | image.img2img | passed | 保留方块与数字布局，应用暖金橙色光照 |
| gpt-5.4-nano | image.img2img | passed | 保留数字及光晕构图，数字和右上侧出现暖色傍晚光照 |
| gpt-5.5 | image.img2img | passed | 保留中心方块与数字，应用暖金色光照和投影 |
| gpt-5.6-sol | image.img2img | passed | 保留主构图，背景和数字呈暖金色 |
| gpt-5.6-terra | image.img2img | passed | 保留圆角方块和数字，应用暖色低角度光照和投影 |
| gpt-6-luna | image.img2img | passed | 保留方块及黑色数字的布局，应用暖橙金色光照 |
| gpt-6-sol | image.img2img | passed | 保留方块尺寸位置和数字，背景及数字呈暖橙金色 |

- 24 个唯一 AICC 任务：16 个目标、8 个图生图源图，均 Terminal/Succeeded；幂等回读不计为新增上游推理。
- 16 个目标均通过持久 usage 与 exact-model/ProviderInstance 路由归因审计。后台提交、查询和结果读取成功，没有因 SN 异步接口缺口 skip，也未降级同步调用。
- 24 次成功上游后台提交的未知费用估算为 $0.24，SN 实际 USD 费用未知。轮询、下载和幂等 RPC 回读不计为新推理。
- 21 个生成输出对象已清理，临时 SN 设置和运行时实例已恢复；制品只在本地保留复核副本，不提交 Git。

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 21 generated output object(s)
