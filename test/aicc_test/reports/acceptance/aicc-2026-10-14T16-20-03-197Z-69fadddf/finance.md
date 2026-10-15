# AICC Test Financial Report

- Currency: USD
- Budget: $1.000000
- Planned maximum: 2 calls / $0.020000
- Executed calls: 2
- Known actual cost: $0.000035
- Raw cost before credit: $0.000035
- Credit applied: $0.000000
- Unknown-cost estimated exposure: $0.010000
- Total exposure: $0.010035
- Remaining budget: $0.989965
- Unknown-cost calls: 1
- Budget exceeded: false

## By provider

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| claude | 1 | 222 | 25 | 247 | 0 | 0.000035 | 0.000000 | 0.000035 | 0.000000 | 0 |
| judge | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |

## By provider instance

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| claude/claude-t2 | 1 | 222 | 25 | 247 | 0 | 0.000035 | 0.000000 | 0.000035 | 0.000000 | 0 |
| judge/claude-t2 | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |

## By exact model

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| claude-haiku-5-5@claude-t2 | 2 | 222 | 25 | 247 | 0 | 0.000035 | 0.000000 | 0.000035 | 0.010000 | 1 |

## By API type

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| llm | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| vision.ocr | 1 | 222 | 25 | 247 | 0 | 0.000035 | 0.000000 | 0.000035 | 0.000000 | 0 |

## By case

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| t2.claude.claude-t2.claude-haiku-5-5.vision.ocr | 1 | 222 | 25 | 247 | 0 | 0.000035 | 0.000000 | 0.000035 | 0.000000 | 0 |
| t2.claude.claude-t2.claude-haiku-5-5.vision.ocr.judge | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
