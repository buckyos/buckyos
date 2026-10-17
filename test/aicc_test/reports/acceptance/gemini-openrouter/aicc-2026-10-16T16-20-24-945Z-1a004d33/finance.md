# AICC Test Financial Report

- Currency: USD
- Budget: $0.100000
- Planned maximum: 4 calls / $0.040000
- Executed calls: 4
- Known actual cost: $0.010181
- Raw cost before credit: $0.010181
- Credit applied: $0.000000
- Unknown-cost estimated exposure: $0.010000
- Total exposure: $0.020181
- Remaining budget: $0.079819
- Unknown-cost calls: 1
- Budget exceeded: false

## By provider

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| openrouter | 4 | 2247 | 579 | 2826 | 0 | 0.010181 | 0.000000 | 0.010181 | 0.010000 | 1 |

## By provider instance

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| openrouter/openrouter-t2 | 4 | 2247 | 579 | 2826 | 0 | 0.010181 | 0.000000 | 0.010181 | 0.010000 | 1 |

## By exact model

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| google/gemini-3.5-transcribe@openrouter-t2 | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| google/gemini-4-argon@openrouter-t2 | 3 | 2247 | 579 | 2826 | 0 | 0.010181 | 0.000000 | 0.010181 | 0.000000 | 0 |

## By API type

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| audio.asr | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| llm | 1 | 48 | 15 | 63 | 0 | 0.000244 | 0.000000 | 0.000244 | 0.000000 | 0 |
| vision.caption | 1 | 1102 | 395 | 1497 | 0 | 0.006092 | 0.000000 | 0.006092 | 0.000000 | 0 |
| vision.ocr | 1 | 1097 | 169 | 1266 | 0 | 0.003845 | 0.000000 | 0.003845 | 0.000000 | 0 |

## By case

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| t2.openrouter.openrouter-t2.google-gemini-3.5-transcribe.audio.asr | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.openrouter.openrouter-t2.google-gemini-4-argon.llm | 1 | 48 | 15 | 63 | 0 | 0.000244 | 0.000000 | 0.000244 | 0.000000 | 0 |
| t2.openrouter.openrouter-t2.google-gemini-4-argon.vision.caption | 1 | 1102 | 395 | 1497 | 0 | 0.006092 | 0.000000 | 0.006092 | 0.000000 | 0 |
| t2.openrouter.openrouter-t2.google-gemini-4-argon.vision.ocr | 1 | 1097 | 169 | 1266 | 0 | 0.003845 | 0.000000 | 0.003845 | 0.000000 | 0 |
