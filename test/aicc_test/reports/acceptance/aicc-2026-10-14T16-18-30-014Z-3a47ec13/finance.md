# AICC Test Financial Report

- Currency: USD
- Budget: $1.000000
- Planned maximum: 4 calls / $0.040000
- Executed calls: 4
- Known actual cost: $0.000250
- Raw cost before credit: $0.000250
- Credit applied: $0.000000
- Unknown-cost estimated exposure: $0.020000
- Total exposure: $0.020250
- Remaining budget: $0.979750
- Unknown-cost calls: 2
- Budget exceeded: false

## By provider

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| claude | 2 | 656 | 368 | 1024 | 0 | 0.000250 | 0.000000 | 0.000250 | 0.000000 | 0 |
| judge | 2 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 2 |

## By provider instance

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| claude/claude-t2 | 2 | 656 | 368 | 1024 | 0 | 0.000250 | 0.000000 | 0.000250 | 0.000000 | 0 |
| judge/claude-t2 | 2 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 2 |

## By exact model

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| claude-haiku-5-5@claude-t2 | 4 | 656 | 368 | 1024 | 0 | 0.000250 | 0.000000 | 0.000250 | 0.020000 | 2 |

## By API type

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| llm | 2 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 2 |
| vision.caption | 1 | 434 | 343 | 777 | 0 | 0.000215 | 0.000000 | 0.000215 | 0.000000 | 0 |
| vision.ocr | 1 | 222 | 25 | 247 | 0 | 0.000035 | 0.000000 | 0.000035 | 0.000000 | 0 |

## By case

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| t2.claude.claude-t2.claude-haiku-5-5.vision.caption | 1 | 434 | 343 | 777 | 0 | 0.000215 | 0.000000 | 0.000215 | 0.000000 | 0 |
| t2.claude.claude-t2.claude-haiku-5-5.vision.caption.judge | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.claude.claude-t2.claude-haiku-5-5.vision.ocr | 1 | 222 | 25 | 247 | 0 | 0.000035 | 0.000000 | 0.000035 | 0.000000 | 0 |
| t2.claude.claude-t2.claude-haiku-5-5.vision.ocr.judge | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
