# OpenRouter Gemini medium 线上回归

- 日期：2026-10-16
- 渠道：OpenRouter Responses API
- 请求参数：`reasoning.effort=medium`
- Judge：未启用（参数兼容性回归，不做语义判定）

| 模型 | HTTP | 结果 |
| --- | ---: | --- |
| `google/gemini-3.8-flash` | 200 | Provider 已接受 `medium`，保留真实 `medium` 映射 |
| `google/gemini-2.5-flash` | 200 | Provider 已接受 `medium`，保留真实 `medium` 映射 |

本轮只验证两个指定 medium 变体，没有重复执行既有 T2 模型/API 矩阵。
