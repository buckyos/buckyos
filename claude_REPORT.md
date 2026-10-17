# AICC Claude T1 / T1.5 / T2 测试报告

## 最终汇总

| 层级 | 最终结果 | 汇总 |
| --- | --- | --- |
| T1 | 通过 | 最终回归 192/192；manifest 130/130，provider 分支 127/127，路由暴露 62/62 |
| T1.5 | 通过 | Claude 最终回归 115/115；真实 Provider 调用 0 |
| T2 | 通过 | 修改矩阵 `claude/claude-haiku-5-5`：LLM、OCR、caption 共 3/3 passed，0 failed/review |

历史全量 T2 的唯一复核项仍为 `claude-sonnet-4-5-20250929` 的 `vision.ocr`：模型输出未包含图片标记 `4827`，归类为模型输出质量问题，不是产品或协议失败。本次修改矩阵复核已全部通过。

本报告链接的 Claude T2 轮次合计产生 96 次真实调用（含 Judge 与诊断调用）。已知实际费用为 USD 0.1849512，无法取得实际费用的历史调用估算为 USD 0.06，总费用敞口为 USD 0.2449512。本次最终定向轮次为 5 次调用，实际费用 USD 0.000838。

T1、T1.5 未调用真实 Provider；测试资源清理检查通过。

## T1 各轮报告

| 轮次 | 结果 | 详细报告 |
| --- | --- | --- |
| 1 | 184 passed，5 failed | [summary.md](test/aicc_test/reports/acceptance/aicc-t1-2026-10-14T15-17-41-222Z-91f6835b/summary.md) |
| 2 | 5 passed | [summary.md](test/aicc_test/reports/acceptance/aicc-t1-2026-10-14T15-23-06-870Z-07c17e8c/summary.md) |
| 3 | 189 passed | [summary.md](test/aicc_test/reports/acceptance/aicc-t1-2026-10-14T15-25-08-007Z-e1e0790a/summary.md) |
| 4（最终全量） | 189 passed | [summary.md](test/aicc_test/reports/acceptance/aicc-t1-2026-10-14T16-29-12-817Z-3c7376e6/summary.md) |
| 5（review 后最终回归） | 192 passed | [summary.md](test/aicc_test/reports/acceptance/aicc-t1-2026-10-17T14-31-31-335Z-cd7511eb/summary.md) |

## T1.5 各轮报告

| 轮次 | 范围与结果 | 详细报告 |
| --- | --- | --- |
| 1 | 前置检查：1 failed | [summary.md](test/aicc_test/reports/acceptance/t15-20261014153006-724100/summary.md) |
| 2（全量） | 全 Provider：1907 passed | [summary.md](test/aicc_test/reports/acceptance/t15-20261014153034-724670/summary.md) |
| 3 | Claude：115 passed | [summary.md](test/aicc_test/reports/acceptance/t15-20261014160925-778473/summary.md) |
| 4 | Claude：115 passed | [summary.md](test/aicc_test/reports/acceptance/t15-20261014161649-791370/summary.md) |
| 5（最终回归） | Claude：115 passed | [summary.md](test/aicc_test/reports/acceptance/t15-20261014162142-798261/summary.md) |
| 6（review 后最终回归） | Claude：115 passed | [summary.md](test/aicc_test/reports/acceptance/t15-20261017143615-2286271/summary.md) |

## T2 各轮报告

| 轮次 | 范围与结果 | 详细报告 |
| --- | --- | --- |
| 1 | Dry-run：2 mismatch，36 skipped | [summary.md](test/aicc_test/reports/acceptance/aicc-2026-10-14T16-02-17-344Z-fb71e32d/summary.md) |
| 2 | Dry-run：0 mismatch，42 skipped | [summary.md](test/aicc_test/reports/acceptance/aicc-2026-10-14T16-10-42-737Z-7ac40134/summary.md) |
| 3 | 定向验证：1 passed，1 review，1 failed | [summary.md](test/aicc_test/reports/acceptance/aicc-2026-10-14T16-11-01-145Z-b852b041/summary.md) |
| 4 | 定向验证：1 review | [summary.md](test/aicc_test/reports/acceptance/aicc-2026-10-14T16-18-05-160Z-9a7d8375/summary.md) |
| 5 | Judge 诊断：2 failed | [summary.md](test/aicc_test/reports/acceptance/aicc-2026-10-14T16-18-30-014Z-3a47ec13/summary.md) |
| 6 | Judge 诊断：1 failed | [summary.md](test/aicc_test/reports/acceptance/aicc-2026-10-14T16-19-20-350Z-82d14546/summary.md) |
| 7 | Judge 诊断：1 failed | [summary.md](test/aicc_test/reports/acceptance/aicc-2026-10-14T16-20-03-197Z-69fadddf/summary.md) |
| 8 | Claude Haiku 5.5 vision：2 passed | [summary.md](test/aicc_test/reports/acceptance/aicc-2026-10-14T16-23-02-488Z-d8d2d61e/summary.md) |
| 9（全量） | 40 passed，1 review，1 failed | [summary.md](test/aicc_test/reports/acceptance/aicc-2026-10-14T16-23-32-070Z-bd567212/summary.md) |
| 10（定向复核） | caption：1 passed | [summary.md](test/aicc_test/reports/acceptance/aicc-2026-10-14T16-28-41-038Z-87d240be/summary.md) |
| 11（修改矩阵） | Haiku 5.5：1 passed，2 review；preflight 13 failed | [summary.md](test/aicc_test/reports/acceptance/aicc-2026-10-17T14-37-43-530Z-e46812f8/summary.md) |
| 12（最终定向复核） | Haiku 5.5：3 passed | [summary.md](test/aicc_test/reports/acceptance/aicc-2026-10-17T14-38-16-038Z-f693e93a/summary.md) |
