# Google Gemini Argon / Transcribe 验收报告

## 最终汇总

| 层级 | 覆盖 | 最终结果 | Cleanup |
| --- | --- | --- | --- |
| T1.5 | Google Gemini 与 OpenRouter 的 Argon / Transcribe 全量目标协议 42 项；Google `gemini-4-argon` 同步 Interactions 定向回归 6 项 | 全量 42/42 通过；最新定向回归 6/6 通过 | 通过 |
| T2 | Google `gemini-4-argon` LLM、ASR、Caption、Detect、OCR、Segment，共 6 项 | 6/6 调用成功，人工语义复核全部通过；无协议错误或确认的产品缺陷 | 通过 |

最新 Google T2 未启用 Judge。六项均经 `interactions.create` 成功完成并返回 usage：LLM 精确返回验收标记，ASR 正确转写测试编号 4827，OCR 识别 `BUCKYOS-DV-4827`，Caption、Detect 和 Segment 输出均与测试图像一致。已知实际费用为 USD 0.000246；其余五项未返回可归因费用，按测试预算计入 USD 0.050000 未知费用估算敞口，总敞口 USD 0.050246。

## 详细报告

- [T1.5 第 1 轮：启动门禁失败](reports/acceptance/gemini-openrouter/t15-20261016160803-453890/summary.md)
- [T1.5 第 2 轮：39 通过，5 失败](reports/acceptance/gemini-openrouter/t15-20261016160923-455797/summary.md)
- [T1.5 最终轮：42/42 通过](reports/acceptance/gemini-openrouter/t15-20261016161641-467190/summary.md)
- [Google Gemini T2 旧轮次：2 项待复核，5 项失败](reports/acceptance/gemini-openrouter/aicc-2026-10-16T16-19-26-107Z-793e8527/summary.md)
- [Google Gemini T2 旧轮次费用](reports/acceptance/gemini-openrouter/aicc-2026-10-16T16-19-26-107Z-793e8527/finance.md)
- [Argon T1.5 旧运行时轮次：0/6 通过](reports/acceptance/gemini-openrouter/t15-20261016163905-499397/summary.md)
- [Argon T2 无真实调用预检：6 项 skipped](reports/acceptance/gemini-openrouter/aicc-2026-10-16T16-41-34-363Z-b78ad0ad/summary.md)
- [Argon T1.5 最新轮次：6/6 通过](reports/acceptance/gemini-openrouter/t15-20261016164540-509753/summary.md)
- [Argon T2 最新轮次：6 项调用成功并完成复核](reports/acceptance/gemini-openrouter/aicc-2026-10-16T16-46-10-273Z-0b845c41/summary.md)
- [Argon T2 最新轮次费用](reports/acceptance/gemini-openrouter/aicc-2026-10-16T16-46-10-273Z-0b845c41/finance.md)
