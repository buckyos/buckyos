# AICC Test Financial Report

- Currency: USD
- Budget: $0.250000
- Planned maximum: 16 calls / $0.160000
- Executed calls: 16
- Known actual cost: $0.001842
- Raw cost before credit: $0.001842
- Credit applied: $0.000000
- Unknown-cost estimated exposure: $0.090000
- Total exposure: $0.091842
- Remaining budget: $0.158158
- Unknown-cost calls: 9
- Budget exceeded: false

## By provider

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| kimi | 6 | 1123 | 1247 | 2370 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.060000 | 6 |
| minimax | 10 | 1337 | 1415 | 2752 | 0 | 0.001842 | 0.000000 | 0.001842 | 0.030000 | 3 |

## By provider instance

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| kimi/kimi-t2 | 6 | 1123 | 1247 | 2370 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.060000 | 6 |
| minimax/minimax-t2 | 10 | 1337 | 1415 | 2752 | 0 | 0.001842 | 0.000000 | 0.001842 | 0.030000 | 3 |

## By exact model

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| kimi-k2.6@kimi-t2 | 3 | 671 | 788 | 1459 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.030000 | 3 |
| kimi-k2.7-code@kimi-t2 | 3 | 452 | 459 | 911 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.030000 | 3 |
| MiniMax-M2.1-highspeed@minimax-t2 | 1 | 51 | 137 | 188 | 0 | 0.000359 | 0.000000 | 0.000359 | 0.000000 | 0 |
| MiniMax-M2.1@minimax-t2 | 1 | 51 | 99 | 150 | 0 | 0.000134 | 0.000000 | 0.000134 | 0.000000 | 0 |
| MiniMax-M2.5-highspeed@minimax-t2 | 1 | 56 | 179 | 235 | 0 | 0.000463 | 0.000000 | 0.000463 | 0.000000 | 0 |
| MiniMax-M2.5@minimax-t2 | 1 | 56 | 151 | 207 | 0 | 0.000198 | 0.000000 | 0.000198 | 0.000000 | 0 |
| MiniMax-M2.7-highspeed@minimax-t2 | 1 | 56 | 85 | 141 | 0 | 0.000238 | 0.000000 | 0.000238 | 0.000000 | 0 |
| MiniMax-M2.7@minimax-t2 | 1 | 56 | 164 | 220 | 0 | 0.000214 | 0.000000 | 0.000214 | 0.000000 | 0 |
| MiniMax-M2@minimax-t2 | 1 | 54 | 183 | 237 | 0 | 0.000236 | 0.000000 | 0.000236 | 0.000000 | 0 |
| MiniMax-M3@minimax-t2 | 3 | 957 | 417 | 1374 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.030000 | 3 |

## By API type

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| llm | 10 | 602 | 1167 | 1769 | 0 | 0.001842 | 0.000000 | 0.001842 | 0.030000 | 3 |
| vision.caption | 3 | 1442 | 1161 | 2603 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.030000 | 3 |
| vision.ocr | 3 | 416 | 334 | 750 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.030000 | 3 |

## By case

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| t2.kimi.kimi-t2.kimi-k2.6.llm | 1 | 22 | 108 | 130 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.kimi.kimi-t2.kimi-k2.6.vision.caption | 1 | 430 | 562 | 992 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.kimi.kimi-t2.kimi-k2.6.vision.ocr | 1 | 219 | 118 | 337 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.kimi.kimi-t2.kimi-k2.7-code.llm | 1 | 22 | 51 | 73 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.kimi.kimi-t2.kimi-k2.7-code.vision.caption | 1 | 430 | 408 | 838 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.kimi.kimi-t2.kimi-k2.7-code.vision.ocr | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.minimax.minimax-t2.minimax-m2.1-highspeed.llm | 1 | 51 | 137 | 188 | 0 | 0.000359 | 0.000000 | 0.000359 | 0.000000 | 0 |
| t2.minimax.minimax-t2.minimax-m2.1.llm | 1 | 51 | 99 | 150 | 0 | 0.000134 | 0.000000 | 0.000134 | 0.000000 | 0 |
| t2.minimax.minimax-t2.minimax-m2.5-highspeed.llm | 1 | 56 | 179 | 235 | 0 | 0.000463 | 0.000000 | 0.000463 | 0.000000 | 0 |
| t2.minimax.minimax-t2.minimax-m2.5.llm | 1 | 56 | 151 | 207 | 0 | 0.000198 | 0.000000 | 0.000198 | 0.000000 | 0 |
| t2.minimax.minimax-t2.minimax-m2.7-highspeed.llm | 1 | 56 | 85 | 141 | 0 | 0.000238 | 0.000000 | 0.000238 | 0.000000 | 0 |
| t2.minimax.minimax-t2.minimax-m2.7.llm | 1 | 56 | 164 | 220 | 0 | 0.000214 | 0.000000 | 0.000214 | 0.000000 | 0 |
| t2.minimax.minimax-t2.minimax-m2.llm | 1 | 54 | 183 | 237 | 0 | 0.000236 | 0.000000 | 0.000236 | 0.000000 | 0 |
| t2.minimax.minimax-t2.minimax-m3.llm | 1 | 178 | 10 | 188 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.minimax.minimax-t2.minimax-m3.vision.caption | 1 | 582 | 191 | 773 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.minimax.minimax-t2.minimax-m3.vision.ocr | 1 | 197 | 216 | 413 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
