# AICC Test Financial Report

- Currency: USD
- Budget: $0.120000
- Planned maximum: 8 calls / $0.080000
- Executed calls: 7
- Known actual cost: $0.000000
- Raw cost before credit: $0.000000
- Credit applied: $0.000000
- Unknown-cost estimated exposure: $0.070000
- Total exposure: $0.070000
- Remaining budget: $0.050000
- Unknown-cost calls: 7
- Budget exceeded: false

## By provider

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| judge | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| kimi | 4 | 1079 | 1319 | 2398 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.040000 | 4 |
| minimax | 2 | 1855 | 126 | 1981 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 2 |

## By provider instance

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| judge/minimax-t2 | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| kimi/kimi-t2 | 4 | 1079 | 1319 | 2398 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.040000 | 4 |
| minimax/minimax-t2 | 2 | 1855 | 126 | 1981 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 2 |

## By exact model

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| kimi-k2.6@kimi-t2 | 2 | 430 | 808 | 1238 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 2 |
| kimi-k2.7-code@kimi-t2 | 2 | 649 | 511 | 1160 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 2 |
| MiniMax-M3@minimax-t2 | 3 | 1855 | 126 | 1981 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.030000 | 3 |

## By API type

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| llm | 3 | 1855 | 126 | 1981 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.030000 | 3 |
| vision.caption | 2 | 860 | 1237 | 2097 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 2 |
| vision.ocr | 2 | 219 | 82 | 301 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 2 |

## By case

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| t2.kimi.kimi-t2.kimi-k2.6.vision.caption | 1 | 430 | 808 | 1238 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.kimi.kimi-t2.kimi-k2.6.vision.caption.judge | 1 | 966 | 61 | 1027 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.kimi.kimi-t2.kimi-k2.6.vision.ocr | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.kimi.kimi-t2.kimi-k2.7-code.vision.caption | 1 | 430 | 429 | 859 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.kimi.kimi-t2.kimi-k2.7-code.vision.caption.judge | 1 | 889 | 65 | 954 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.kimi.kimi-t2.kimi-k2.7-code.vision.ocr | 1 | 219 | 82 | 301 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.kimi.kimi-t2.kimi-k2.7-code.vision.ocr.judge | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
