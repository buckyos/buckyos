# AICC DeepSeek / Kimi / MiniMax 最终测试报告

测试日期：2026-10-08

测试环境：Ubuntu，BuckyOS `/opt/buckyos`。凭证未写入报告。

## 最终结论

| 层级 | 覆盖范围 | 最终结果 |
|---|---|---:|
| T1 | AICC 全量静态、路由与配置契约 | 178 / 178 通过 |
| T1.5 | 新增模型前的 Provider 全量回归基线 | 1605 / 1605 通过 |
| T1.5 | DeepSeek 全量 | 30 / 30 通过 |
| T1.5 | Kimi 本次新增模型：`kimi-k3`、`kimi-k2.7-code-highspeed` | 26 / 26 通过 |
| T1.5 | MiniMax 全量 | 82 / 82 通过 |
| T2 | DeepSeek：`deepseek-flash`、`deepseek-v4-pro` | 2 passed，2 review，0 failed |
| T2 | Kimi 全部已测模型，共 12 个模型/API 单元 | 8 passed，4 review，0 failed |
| T2 | MiniMax 全部已测模型，共 19 个模型/API 单元 | 9 passed，10 review，0 failed |

DeepSeek 的两个 LLM 单元已通过真实调用；`deepseek-flash / vision.ocr` 与 `deepseek-flash / vision.caption` 均已成功完成真实调用。由于本轮未启用 Judge，两个视觉单元最终状态保留为人工语义复核 `review`。DeepSeek 全部四个单元无失败、无确认的产品缺陷。

Kimi 的旧模型 `kimi-k2.6`、`kimi-k2.7-code` 共六个 T2 单元最终全部通过。新增的 `kimi-k3` 与 `kimi-k2.7-code-highspeed` 均已进入官方账号库存并通过 T1.5；充值后的六个真实 T2 单元全部成功调用，其中两个 LLM 单元通过，四个视觉单元因未启用 Judge 保留为人工语义复核 `review`。Kimi 全部已测单元无失败、无账号权限限制、无确认的 AICC 产品缺陷。

MiniMax 旧模型矩阵共十个 T2 单元，最终结果为九个通过、一个人工语义复核。新增库存包含 `asr-1.0`、`speech-2.8-hd`、`speech-2.8-turbo`、`image-01`、`MiniMax-H3`、`MiniMax-H3-Max`；新增九个 T2 单元均获得成功协议响应，图片、音频和视频制品均通过下载与格式校验。由于新增模型复验未启用 Judge，这九个单元最终状态保留为人工语义复核 `review`。MiniMax 全部已测单元无失败、无确认的产品缺陷。

所有最终 T1.5/T2 报告的清理状态均为 passed。

## 详细报告链接

### T1

- [AICC T1 全量报告](../reports/acceptance/aicc-t1-2026-10-08T04-31-37-310Z-9bea28a0/summary.md)

### T1.5

- [新增模型前的 Provider 全量回归基线：1605 / 1605](../reports/acceptance/t15-20261008050816-1602985/summary.md)
- [DeepSeek 30 / 30](../reports/acceptance/t15-20261008102309-22405/summary.md)
- [Kimi 新增模型 26 / 26](../reports/acceptance/t15-20261008102337-22781/summary.md)
- [MiniMax 82 / 82](../reports/acceptance/t15-20261008104624-67760/summary.md)

### T2

- [Kimi 旧模型 LLM 与 MiniMax 旧模型 LLM 最终结果](../reports/acceptance/aicc-2026-10-08T05-44-41-155Z-e37ceeba/summary.md)
- [Kimi 旧模型 vision.caption 最终结果](../reports/acceptance/aicc-2026-10-08T05-54-06-572Z-2d9bf678/summary.md)
- [Kimi 旧模型 vision.ocr 最终结果](../reports/acceptance/aicc-2026-10-08T06-02-49-682Z-0dc74172/summary.md)
- [MiniMax 旧模型 M3 视觉最终结果](../reports/acceptance/aicc-2026-10-08T06-04-51-424Z-fb8605b0/summary.md)
- [DeepSeek 与首轮目标矩阵结果](../reports/acceptance/aicc-2026-10-08T10-27-44-863Z-53cd8c14/summary.md)
- [DeepSeek Flash vision.caption 最终复验结果](../reports/acceptance/aicc-2026-10-08T14-04-29-790Z-ded511de/summary.md)
- [Kimi K3 与 K2.7 Code HighSpeed 六个单元最终结果](../reports/acceptance/aicc-2026-10-08T14-23-08-656Z-e504c9eb/summary.md)
- [MiniMax 新增模型七个单元最终结果](../reports/acceptance/aicc-2026-10-08T10-49-33-585Z-a00bd81c/summary.md)
- [MiniMax 图生视频两个单元最终结果](../reports/acceptance/aicc-2026-10-08T10-53-03-815Z-e9a08ce1/summary.md)
