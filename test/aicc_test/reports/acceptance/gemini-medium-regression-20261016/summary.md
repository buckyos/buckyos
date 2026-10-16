# Gemini medium 线上回归

- 日期：2026-10-16
- 渠道：Google Gemini 官方 Interactions API
- 请求参数：`generation_config.thinking_level=medium`
- Judge：未启用（参数兼容性回归，不做语义判定）
- 官方依据：[Gemini Thinking](https://ai.google.dev/gemini-api/docs/thinking)

| 模型 | HTTP | 结果 |
| --- | ---: | --- |
| `gemini-3.8-flash` | 200 | Provider 已接受 `medium`，保留真实 `medium` 映射 |
| `gemini-2.5-flash` | 400 | Provider 返回仅允许 `high, low`；官网仍声明支持 `medium`，AICC 保留 `reasoning-medium` 并将本渠道底层参数兼容映射为 `high` |

本轮只验证两个指定 medium 变体，没有重复执行既有 T2 模型/API 矩阵。
