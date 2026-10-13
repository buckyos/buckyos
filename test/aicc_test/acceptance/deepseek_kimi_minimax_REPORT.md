# AICC DeepSeek / Kimi / MiniMax 最终测试报告

测试日期：2026-10-13

测试环境：Ubuntu，BuckyOS `/opt/buckyos`。凭证未写入报告。

## 最终汇总

| 层级 | 覆盖范围 | 最终结果 |
|---|---|---:|
| T1 | AICC 全量静态、路由与配置契约 | 189 / 189 通过 |
| T1.5 | Provider 全量回归基线 | 1605 / 1605 通过 |
| T1.5 | DeepSeek 全量 | 30 / 30 通过 |
| T1.5 | Kimi 官方 API 当前四个模型 | 35 / 35 通过 |
| T1.5 | MiniMax 全量协议矩阵 | 100 / 100 通过 |
| T2 | DeepSeek：`deepseek-flash`、`deepseek-v4-pro` | 2 passed，2 review，0 failed |
| T2 | Kimi：4 个模型 × `llm` / `vision.caption` / `vision.ocr` | 12 / 12 通过 |
| T2 | MiniMax：22 个模型/API 单元 | 9 passed，10 review，3 provider_restricted，0 failed |

MiniMax 参数已按官方资料校准：M3.1 Flash Preview 使用 1M context、524288 最大输出、强制 thinking，并支持 `low` / `medium` / `high` / `xhigh` / `max` effort；M3 使用 1M context、524288 最大输出及可关闭/自适应 thinking；M2.x 使用 204800 context、强制 thinking，不暴露 effort。M3/M3.1 默认 `temperature=1`、`top_p=0.95`，M2.x 默认 `temperature=1`、`top_p=0.9`。

H3 与 H3-Max 的默认值和合法范围均由 MiniMax provider 配置注入：H3 为 4–15 秒、`768P` / `2K`，H3-Max 为 5–15 秒、`480P` / `768P`。运行时协议实现不再按模型名选择时长、分辨率或默认参数分支。

M3.1 按官方限定作为 M Plan / MiniMax Code 预览模型 exact-only 暴露。本轮所用中国区 API Key 的官方 `/anthropic/v1/models` 返回 M3 与 M2.x，不返回 M3.1，因此其 `llm`、`vision.caption`、`vision.ocr` 三个缺失 T2 单元均归类为 `provider_restricted`；未向无权限的推理端点发起调用，真实调用数与费用均为 0。

Kimi 官方 API 模型目录和账号 `/v1/models` 当前提供 `kimi-k3`、`kimi-k2.7-code`、`kimi-k2.7-code-highspeed`、`kimi-k2.6`。`kimi-k2.8-preview` 不在 Kimi API 账号库存中，因此未作为 `kimi` Provider 的 T1.5/T2 模型执行。

DeepSeek 与 MiniMax 的既有真实调用单元均无 failed；未启用 Judge 的视觉或媒体单元保留为 review。

## 详细报告链接

### T1

- [最终全量：189 / 189](../reports/acceptance/aicc-t1-2026-10-13T10-23-24-980Z-18a39645/summary.md)

### T1.5

- [Provider 全量：1605 / 1605](../reports/acceptance/t15-20261010101158-3080107/summary.md)
- [DeepSeek：30 / 30](../reports/acceptance/t15-20261008102309-22405/summary.md)
- [Kimi 官方四模型：35 / 35](../reports/acceptance/t15-20261010160418-3615574/summary.md)
- [MiniMax 最终协议回归：100 / 100](../reports/acceptance/t15-20261013103926-850006/summary.md)

### T2

- [DeepSeek 与 Kimi / MiniMax 首轮目标矩阵](../reports/acceptance/aicc-2026-10-08T10-27-44-863Z-53cd8c14/summary.md)
- [DeepSeek Flash vision.caption 最终结果](../reports/acceptance/aicc-2026-10-08T14-04-29-790Z-ded511de/summary.md)
- [Kimi 官方四模型最终结果：12 / 12](../reports/acceptance/aicc-2026-10-10T16-05-07-946Z-b2b047ea/summary.md)
- [Kimi 旧模型与 MiniMax M2.x / M3 LLM](../reports/acceptance/aicc-2026-10-08T05-44-41-155Z-e37ceeba/summary.md)
- [MiniMax M3 视觉](../reports/acceptance/aicc-2026-10-08T06-04-51-424Z-fb8605b0/summary.md)
- [MiniMax 语音、图像与 H3/H3-Max 七个单元](../reports/acceptance/aicc-2026-10-08T10-49-33-585Z-a00bd81c/summary.md)
- [MiniMax H3/H3-Max 图生视频两个单元](../reports/acceptance/aicc-2026-10-08T10-53-03-815Z-e9a08ce1/summary.md)
- [MiniMax M3.1 三个缺失单元：3 provider_restricted，0 failed，0 real calls](../reports/acceptance/aicc-2026-10-13T10-48-36-117Z-d8b6d2cd/summary.md)
