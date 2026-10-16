# AICC Qwen T1 / T1.5 / T2 测试报告

## 最终汇总

| 层级 | 最终覆盖 | 最终结果 | Cleanup |
| --- | --- | --- | --- |
| T1 | 全量 184 项 | 184/184 通过 | 通过 |
| T1.5 | 全量 1382 项及 Qwen 增量 236 项 | 跨轮累计 1382/1382 通过；最终 Qwen 回归 236/236 通过 | 通过 |
| T2 | Qwen 官方目录内 39 个物理模型、61 个不重复的 Model × API 单元格 | 跨轮累计 61/61 通过 | 各轮 Cleanup 通过 |

T2 的 61 个通过单元格覆盖 Qwen 原生文本、视觉、图像和视频模型，阿里云百炼部署的 DeepSeek、GLM、Kimi、MiniMax 文本模型，以及带 `vanchin/`、`ZHIPU/`、`kimi/`、`MiniMax/` 命名空间的厂商直供文本和视觉模型。Qwen 官方目录未提供 Doubao 模型 ID，因此未将 Doubao 纳入 Qwen 聚合平台测试。实际调用费用以各轮报告记录的 Provider 币种为准；未知的美元折算额不记作零。

这些聚合模型并非都只有 LLM 能力：DeepSeek v4.1 Flash、GLM 5.3 Flash、Kimi K2.6/K2.7 Code/K2.7 Code Highspeed/K3、MiniMax M3 的视觉能力均已通过 T2。原厂能力由对应 `*.model.json` 定义；Qwen 的模型 ID 映射、调用协议、价格、Qwen 专属 variants 和差异化限制由 `qwen.provider.json` 定义。Kimi 与 MiniMax 的 `*.model.json` 未保留 Qwen 平台专属参数。

Qwen `/models` 对当前有效 API Key 返回 262 个模型，并包含当前账号未开通的 `xiaomi/mimo-v2.5-pro`，因此该接口不是逐模型授权清单。厂商直供模型由 WebUI 显式选择；Qwen 自有的已知可用基础模型继续自动发现。`tongyi-xiaomi-analysis-flash` 和 `tongyi-xiaomi-analysis-pro` 虽默认提供免费试用，但仓库没有内置 Xiaomi Model Driver，按本次范围未加入 AICC 库存。

## T1 轮次报告

- [第 1 轮：180 通过，4 失败](reports/acceptance/aicc-t1-2026-10-09T09-39-14-345Z-3c3f0381/summary.md)
- [第 2 轮：2 通过，2 失败](reports/acceptance/aicc-t1-2026-10-09T09-46-11-535Z-8b688bbd/summary.md)
- [第 3 轮：0 通过，2 失败](reports/acceptance/aicc-t1-2026-10-09T09-48-41-434Z-02276eaa/summary.md)
- [第 4 轮：0 通过，2 失败](reports/acceptance/aicc-t1-2026-10-09T09-51-15-974Z-b33227c2/summary.md)
- [第 5 轮：0 通过，1 失败](reports/acceptance/aicc-t1-2026-10-09T09-54-34-420Z-533441d0/summary.md)
- [第 6 轮：0 通过，2 失败](reports/acceptance/aicc-t1-2026-10-09T10-03-23-752Z-8da70950/summary.md)
- [第 7 轮：2/2 通过](reports/acceptance/aicc-t1-2026-10-09T10-13-30-904Z-7d30b54c/summary.md)
- [第 8 轮（最终全量）：184/184 通过](reports/acceptance/aicc-t1-2026-10-09T10-15-22-826Z-50dd76a0/summary.md)
- [第 9 轮（最终全量复核）：184/184 通过](reports/acceptance/aicc-t1-2026-10-09T14-28-43-946Z-3cf275be/summary.md)

## T1.5 轮次报告

- [第 1 轮（全量）：1605/1605 通过](reports/acceptance/t15-20261009102046-1386856/summary.md)
- [第 2 轮（全量）：1625/1625 通过](reports/acceptance/t15-20261009111407-1464232/summary.md)
- [第 3 轮（Qwen）：97/97 通过](reports/acceptance/t15-20261009115106-1516149/summary.md)
- [第 4 轮（Qwen）：0 通过，1 失败](reports/acceptance/t15-20261009121833-1554414/summary.md)
- [第 5 轮（Qwen）：97/97 通过](reports/acceptance/t15-20261009121853-1554923/summary.md)
- [第 6 轮（Qwen）：97 通过，1 失败](reports/acceptance/t15-20261009123034-1571064/summary.md)
- [第 7 轮（Qwen 最终回归）：98/98 通过](reports/acceptance/t15-20261009123531-1577484/summary.md)
- [第 8 轮（Qwen 聚合协议启动门禁）：0 通过，1 失败](reports/acceptance/t15-20261009134028-1667305/summary.md)
- [第 9 轮（Qwen 聚合协议）：12/12 通过](reports/acceptance/t15-20261009134048-1668115/summary.md)
- [第 10 轮（Qwen 聚合库存门禁）：1/1 通过](reports/acceptance/t15-20261009134815-1680607/summary.md)
- [第 11 轮（全量启动门禁）：0 通过，1 失败](reports/acceptance/t15-20261009143337-1758434/summary.md)
- [第 12 轮（全量）：1381 通过，1 失败](reports/acceptance/t15-20261009143349-1758749/summary.md)
- [第 13 轮（Qwen variant 库存门禁）：0 通过，1 失败](reports/acceptance/t15-20261009150156-1797892/summary.md)
- [第 14 轮（Qwen variant 基础模型门禁）：0 通过，1 失败](reports/acceptance/t15-20261009150238-1798918/summary.md)
- [第 15 轮（Qwen 聚合 variant 协议）：112 通过，124 失败](reports/acceptance/t15-20261009150636-1806232/summary.md)
- [第 16 轮（Qwen 最终回归）：236/236 通过](reports/acceptance/t15-20261009151255-1814876/summary.md)
- [第 17 轮（Qwen 最新元数据复核）：236/236 通过](reports/acceptance/t15-20261009153158-1844755/summary.md)
- [第 18 轮（Qwen 最终部署复核）：236/236 通过](reports/acceptance/t15-20261009155732-1885366/summary.md)

## T2 轮次报告

- [第 1 轮：0 通过，59 失败，2 跳过](reports/acceptance/aicc-2026-10-09T10-59-09-267Z-aed62ddf/summary.md)
- [第 2 轮：0 通过，2 跳过](reports/acceptance/aicc-2026-10-09T11-00-01-736Z-5a78ed8f/summary.md)
- [第 3 轮：2/2 通过](reports/acceptance/aicc-2026-10-09T11-00-13-855Z-8d5c179f/summary.md)
- [第 4 轮：1 通过，2 失败](reports/acceptance/aicc-2026-10-09T11-00-51-352Z-338061ef/summary.md)
- [第 5 轮：1 通过，2 待复核](reports/acceptance/aicc-2026-10-09T11-02-33-633Z-a8a4c04e/summary.md)
- [第 6 轮：1 通过，2 失败](reports/acceptance/aicc-2026-10-09T11-03-13-025Z-049beddc/summary.md)
- [第 7 轮：3/3 通过](reports/acceptance/aicc-2026-10-09T11-04-47-713Z-85fa9413/summary.md)
- [第 8 轮：4/4 通过](reports/acceptance/aicc-2026-10-09T11-44-17-580Z-6fe1b262/summary.md)
- [第 9 轮：3 通过，3 失败](reports/acceptance/aicc-2026-10-09T11-55-24-315Z-5d47408b/summary.md)
- [第 10 轮：2 通过，1 失败](reports/acceptance/aicc-2026-10-09T12-23-31-487Z-aa38be64/summary.md)
- [第 11 轮：1/1 通过](reports/acceptance/aicc-2026-10-09T12-25-50-312Z-1f09bab5/summary.md)
- [第 12 轮：0 通过，221 失败，75 跳过](reports/acceptance/aicc-2026-10-09T13-29-10-817Z-24090c20/summary.md)
- [第 13 轮：0 通过，221 失败，75 跳过](reports/acceptance/aicc-2026-10-09T13-29-23-415Z-37321c77/summary.md)
- [第 14 轮（百炼部署聚合模型）：12 通过，11 失败](reports/acceptance/aicc-2026-10-09T13-41-26-696Z-347b78de/summary.md)
- [第 15 轮（失败单元回归）：3/3 通过](reports/acceptance/aicc-2026-10-09T13-48-31-282Z-b86b239b/summary.md)
- [第 16 轮（厂商直供聚合模型）：0 通过，17 失败](reports/acceptance/aicc-2026-10-09T13-48-55-343Z-c08ce3c6/summary.md)
- [第 17 轮（已开通厂商直供文本及多模态）：17 通过，13 待复核，1 失败](reports/acceptance/aicc-2026-10-09T15-38-34-079Z-e05a977d/summary.md)
- [第 18 轮（厂商直供多模态最终复核）：14/14 通过](reports/acceptance/aicc-2026-10-09T15-43-32-579Z-7fa08d7b/summary.md)
