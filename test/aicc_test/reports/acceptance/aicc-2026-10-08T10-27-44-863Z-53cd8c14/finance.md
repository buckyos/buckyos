# AICC Test Financial Report

- Currency: USD
- Budget: $20.000000
- Planned maximum: 19 calls / $0.190000
- Executed calls: 19
- Known actual cost: $0.000222
- Raw cost before credit: $0.000222
- Credit applied: $0.000000
- Unknown-cost estimated exposure: $0.160000
- Total exposure: $0.160222
- Remaining budget: $19.839778
- Unknown-cost calls: 16
- Budget exceeded: false

## By provider

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| deepseek | 4 | 379 | 135 | 514 | 0 | 0.000222 | 0.000000 | 0.000222 | 0.010000 | 1 |
| kimi | 6 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.060000 | 6 |
| minimax | 9 | 0 | 0 | 0 | 2 | 0.000000 | 0.000000 | 0.000000 | 0.090000 | 9 |

## By provider instance

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| deepseek/deepseek-t2 | 4 | 379 | 135 | 514 | 0 | 0.000222 | 0.000000 | 0.000222 | 0.010000 | 1 |
| kimi/kimi-t2 | 6 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.060000 | 6 |
| minimax/minimax-t2 | 9 | 0 | 0 | 0 | 2 | 0.000000 | 0.000000 | 0.000000 | 0.090000 | 9 |

## By exact model

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| asr-1.0@minimax-t2 | 1 | 0 | 0 | 0 | 1 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| deepseek-flash@deepseek-t2 | 3 | 281 | 110 | 391 | 0 | 0.000108 | 0.000000 | 0.000108 | 0.010000 | 1 |
| deepseek-v4-pro@deepseek-t2 | 1 | 98 | 25 | 123 | 0 | 0.000114 | 0.000000 | 0.000114 | 0.000000 | 0 |
| image-01@minimax-t2 | 2 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 2 |
| kimi-k2.7-code-highspeed@kimi-t2 | 3 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.030000 | 3 |
| kimi-k3@kimi-t2 | 3 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.030000 | 3 |
| MiniMax-H3-Max@minimax-t2 | 2 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 2 |
| MiniMax-H3@minimax-t2 | 2 | 0 | 0 | 0 | 1 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 2 |
| speech-2.8-hd@minimax-t2 | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| speech-2.8-turbo@minimax-t2 | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |

## By API type

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| audio.asr | 1 | 0 | 0 | 0 | 1 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| audio.tts | 2 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 2 |
| image.txt2img | 2 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 2 |
| llm | 4 | 143 | 53 | 196 | 0 | 0.000138 | 0.000000 | 0.000138 | 0.020000 | 2 |
| video.img2video | 2 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 2 |
| video.txt2video | 2 | 0 | 0 | 0 | 1 | 0.000000 | 0.000000 | 0.000000 | 0.020000 | 2 |
| vision.caption | 3 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.030000 | 3 |
| vision.ocr | 3 | 236 | 82 | 318 | 0 | 0.000085 | 0.000000 | 0.000085 | 0.020000 | 2 |

## By case

| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| t2.deepseek.deepseek-t2.deepseek-flash.llm | 1 | 45 | 28 | 73 | 0 | 0.000024 | 0.000000 | 0.000024 | 0.000000 | 0 |
| t2.deepseek.deepseek-t2.deepseek-flash.vision.caption | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.deepseek.deepseek-t2.deepseek-flash.vision.ocr | 1 | 236 | 82 | 318 | 0 | 0.000085 | 0.000000 | 0.000085 | 0.000000 | 0 |
| t2.deepseek.deepseek-t2.deepseek-v4-pro.llm | 1 | 98 | 25 | 123 | 0 | 0.000114 | 0.000000 | 0.000114 | 0.000000 | 0 |
| t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.llm | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.vision.caption | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.vision.ocr | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.kimi.kimi-t2.kimi-k3.llm | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.kimi.kimi-t2.kimi-k3.vision.caption | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.kimi.kimi-t2.kimi-k3.vision.ocr | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.minimax.minimax-t2.asr-1.0.audio.asr | 1 | 0 | 0 | 0 | 1 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.minimax.minimax-t2.image-01.image.txt2img | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.minimax.minimax-t2.minimax-h3-max.video.img2video | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.minimax.minimax-t2.minimax-h3-max.video.txt2video | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.minimax.minimax-t2.minimax-h3.video.img2video | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.minimax.minimax-t2.minimax-h3.video.txt2video | 1 | 0 | 0 | 0 | 1 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.minimax.minimax-t2.speech-2.8-hd.audio.tts | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.minimax.minimax-t2.speech-2.8-turbo.audio.tts | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
| t2.prerequisite.generated_artifact.image.txt2img.image-01@minimax-t2 | 1 | 0 | 0 | 0 | 0 | 0.000000 | 0.000000 | 0.000000 | 0.010000 | 1 |
