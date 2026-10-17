# OpenRouter Gemini Argon / Transcribe 验收报告

## 最终汇总

| 层级 | 覆盖 | 最终结果 | Cleanup |
| --- | --- | --- | --- |
| T1.5 | Google Gemini 与 OpenRouter 的 Argon / Transcribe 目标协议，共 42 项 | 42/42 通过 | 通过 |
| T2 | `google/gemini-3.5-transcribe` ASR；`google/gemini-4-argon` LLM、Caption、OCR，共 4 项 | 4 项调用成功并待人工语义复核；无协议错误或产品缺陷 | 通过 |

OpenRouter T2 未启用 Judge。Argon 三项返回的实际费用合计为 USD 0.01018116；Transcribe 未返回可归因费用。

Google Gemini Provider 的最新同步 Interactions 回归不改变 OpenRouter 配置或既有结论；OpenRouter 仍按其 Provider 规则为 `google/gemini-4-argon` 提交 `background=true`。本轮未新增 OpenRouter 真实调用。

## 详细报告

- [T1.5 第 1 轮：启动门禁失败](reports/acceptance/gemini-openrouter/t15-20261016160803-453890/summary.md)
- [T1.5 第 2 轮：39 通过，5 失败](reports/acceptance/gemini-openrouter/t15-20261016160923-455797/summary.md)
- [T1.5 最终轮：42/42 通过](reports/acceptance/gemini-openrouter/t15-20261016161641-467190/summary.md)
- [OpenRouter T2：4 项待复核](reports/acceptance/gemini-openrouter/aicc-2026-10-16T16-20-24-945Z-1a004d33/summary.md)
- [OpenRouter T2 费用](reports/acceptance/gemini-openrouter/aicc-2026-10-16T16-20-24-945Z-1a004d33/finance.md)
