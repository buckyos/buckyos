# AICC Test Financial Report

- Currency: USD
- Budget: $0.300000
- Planned maximum: 13 calls / $0.130000
- Executed calls: 7
- Known actual cost: $0.000000
- Raw cost before credit: $0.000000
- Credit applied: $0.000000
- Unknown-cost estimated exposure: $0.130000
- Total exposure: $0.130000
- Remaining budget: $0.170000
- Unknown-cost calls: 7
- Budget exceeded: false

调用口径：Executed calls=7 表示 AICC 推理任务数，不是 Provider HTTP POST 数。六个 computer-use 任务各有两次请求（request_units 合计 12）；OCR 另有一次后台提交，因此上游推理请求为 13 次。轮询 GET 不计入推理请求。未知费用估算 $0.13，SN 实际扣费未知。

## By provider

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| sn-ai-provider | 7 | 30780 | 591 | 31371 | 12 | 0.000000 | 0.000000 | 0.000000 | 0.130000 | 7 |

## By provider instance

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| sn-ai-provider/sn-ai-provider-default | 7 | 30780 | 591 | 31371 | 12 | 0.000000 | 0.000000 | 0.000000 | 0.130000 | 7 |

## By exact model

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| gpt-5.4-nano@sn-ai-provider-default | 1 | 225 | 23 | 248 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| gpt-5.6-luna@sn-ai-provider-default | 1 | 5084 | 108 | 5192 | 2 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 1 |
| gpt-5.6-sol@sn-ai-provider-default | 1 | 5107 | 102 | 5209 | 2 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 1 |
| gpt-5.6-terra@sn-ai-provider-default | 1 | 5096 | 105 | 5201 | 2 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 1 |
| gpt-6-astra@sn-ai-provider-default | 1 | 5074 | 69 | 5143 | 2 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 1 |
| gpt-6-luna@sn-ai-provider-default | 1 | 5104 | 99 | 5203 | 2 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 1 |
| gpt-6-sol@sn-ai-provider-default | 1 | 5090 | 85 | 5175 | 2 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 1 |

## By API type

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| agent.computer_use | 6 | 30555 | 568 | 31123 | 12 | 0.000000 | 0.000000 | 0.000000 | 0.120000 | 6 |
| vision.ocr | 1 | 225 | 23 | 248 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |

## By case

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-nano.vision.ocr | 1 | 225 | 23 | 248 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-luna.agent.computer_use | 1 | 5084 | 108 | 5192 | 2 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 1 |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.agent.computer_use | 1 | 5107 | 102 | 5209 | 2 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 1 |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.agent.computer_use | 1 | 5096 | 105 | 5201 | 2 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 1 |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.agent.computer_use | 1 | 5074 | 69 | 5143 | 2 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 1 |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.agent.computer_use | 1 | 5104 | 99 | 5203 | 2 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 1 |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.agent.computer_use | 1 | 5090 | 85 | 5175 | 2 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 1 |
