# AICC Test Financial Report

- Currency: USD
- Budget: $0.700000
- Planned maximum: 24 calls / $0.240000
- Executed calls: 24
- Known actual cost: $0.000000
- Raw cost before credit: $0.000000
- Credit applied: $0.000000
- Unknown-cost estimated exposure: $0.240000
- Total exposure: $0.240000
- Remaining budget: $0.460000
- Unknown-cost calls: 24
- Budget exceeded: false

调用口径：本轮覆盖 24 个唯一 AICC 任务（16 个目标、8 个前置生成），全部复用原幂等键；恢复运行产生的幂等 RPC 回读不计为新增模型推理。24 次成功后台提交的未知费用估算为 $0.24，非实际账单；GET 轮询和制品下载不计为推理。

## By provider

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| sn-ai-provider | 24 | 45820 | 19234 | 65054 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.240000 | 24 |

## By provider instance

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| sn-ai-provider/sn-ai-provider-default | 24 | 45820 | 19234 | 65054 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.240000 | 24 |

## By exact model

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| gpt-5.4-mini@sn-ai-provider-default | 2 | 4560 | 148 | 4708 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 2 |
| gpt-5.4-nano@sn-ai-provider-default | 3 | 6222 | 219 | 6441 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.030000 | 3 |
| gpt-5.4-pro@sn-ai-provider-default | 2 | 1921 | 7812 | 9733 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 2 |
| gpt-5.4@sn-ai-provider-default | 3 | 6200 | 197 | 6397 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.030000 | 3 |
| gpt-5.5-pro@sn-ai-provider-default | 1 | 225 | 9387 | 9612 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| gpt-5.5@sn-ai-provider-default | 2 | 4543 | 131 | 4674 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 2 |
| gpt-5.6-sol@sn-ai-provider-default | 2 | 4642 | 230 | 4872 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 2 |
| gpt-5.6-terra@sn-ai-provider-default | 3 | 4846 | 439 | 5285 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.030000 | 3 |
| gpt-6-astra@sn-ai-provider-default | 1 | 1659 | 68 | 1727 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| gpt-6-luna@sn-ai-provider-default | 3 | 6329 | 342 | 6671 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.030000 | 3 |
| gpt-6-sol@sn-ai-provider-default | 2 | 4673 | 261 | 4934 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 2 |

## By API type

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| image.img2img | 8 | 23592 | 1040 | 24632 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.080000 | 8 |
| image.txt2img | 13 | 21553 | 923 | 22476 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.130000 | 13 |
| vision.ocr | 3 | 675 | 17271 | 17946 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.030000 | 3 |

## By case

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| t2.prerequisite.generated_artifact.image.txt2img.gpt-5.4-mini@sn-ai-provider-default | 1 | 1644 | 53 | 1697 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.prerequisite.generated_artifact.image.txt2img.gpt-5.4-nano@sn-ai-provider-default | 1 | 1651 | 60 | 1711 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.prerequisite.generated_artifact.image.txt2img.gpt-5.4@sn-ai-provider-default | 1 | 1640 | 49 | 1689 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.prerequisite.generated_artifact.image.txt2img.gpt-5.5@sn-ai-provider-default | 1 | 1641 | 50 | 1691 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.prerequisite.generated_artifact.image.txt2img.gpt-5.6-sol@sn-ai-provider-default | 1 | 1677 | 86 | 1763 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.prerequisite.generated_artifact.image.txt2img.gpt-5.6-terra@sn-ai-provider-default | 1 | 1659 | 68 | 1727 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.prerequisite.generated_artifact.image.txt2img.gpt-6-luna@sn-ai-provider-default | 1 | 1654 | 63 | 1717 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.prerequisite.generated_artifact.image.txt2img.gpt-6-sol@sn-ai-provider-default | 1 | 1674 | 83 | 1757 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-mini.image.img2img | 1 | 2916 | 95 | 3011 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-nano.image.img2img | 1 | 2926 | 105 | 3031 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-nano.image.txt2img | 1 | 1645 | 54 | 1699 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-pro.image.txt2img | 1 | 1696 | 158 | 1854 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-pro.vision.ocr | 1 | 225 | 7654 | 7879 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4.image.img2img | 1 | 2914 | 93 | 3007 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4.image.txt2img | 1 | 1646 | 55 | 1701 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.5-pro.vision.ocr | 1 | 225 | 9387 | 9612 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.5.image.img2img | 1 | 2902 | 81 | 2983 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.image.img2img | 1 | 2965 | 144 | 3109 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.image.img2img | 1 | 2962 | 141 | 3103 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.vision.ocr | 1 | 225 | 230 | 455 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.image.txt2img | 1 | 1659 | 68 | 1727 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.image.img2img | 1 | 3008 | 203 | 3211 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.image.txt2img | 1 | 1667 | 76 | 1743 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.image.img2img | 1 | 2999 | 178 | 3177 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
