# AICC DeepSeek / Kimi / MiniMax 最终测试报告

测试日期：2026-10-11

测试环境：Ubuntu，BuckyOS `/opt/buckyos`。凭证未写入报告。

## 最终汇总

| 层级 | 覆盖范围 | 最终结果 |
|---|---|---:|
| T1 | AICC 全量静态、路由与配置契约 | 184 / 184 通过 |
| T1.5 | Provider 全量回归基线 | 1605 / 1605 通过 |
| T1.5 | DeepSeek 全量 | 30 / 30 通过 |
| T1.5 | Kimi 官方 API 当前四个模型的协议与视觉覆盖 | 35 / 35 通过 |
| T1.5 | MiniMax 全量 | 82 / 82 通过 |
| T2 | DeepSeek：`deepseek-flash`、`deepseek-v4-pro` | 2 passed，2 review，0 failed |
| T2 | Kimi：4 个模型 × `llm` / `vision.caption` / `vision.ocr` | 12 / 12 通过 |
| T2 | MiniMax 全部已测模型，共 19 个模型/API 单元 | 9 passed，10 review，0 failed |

Kimi 官方 API 模型目录和账号 `/v1/models` 当前提供 `kimi-k3`、`kimi-k2.7-code`、`kimi-k2.7-code-highspeed`、`kimi-k2.6`。`kimi-k2.8-preview` 仅见于独立的 Kimi Code 模型说明，不在 Kimi API 模型目录或账号库存中，因此未作为 `kimi` Provider 的 T1.5/T2 模型执行；它在 Doubao Agent Plan 下的能力归属保持不变。

Kimi 最终 T2 的 12 个业务单元及 8 个视觉语义裁判均通过，20 / 20 次真实调用归因完整；视觉裁判分数为 0.95–1.0。最终报告的配置、凭据与临时资源清理状态为 passed。

DeepSeek 与 MiniMax 的既有最终结论保持不变：所有真实调用单元无 failed，未启用 Judge 的视觉或媒体单元保留为 review。

## 详细报告链接

### T1

- [定向：0 passed，1 failed](../reports/acceptance/aicc-t1-2026-10-10T03-48-08-395Z-4cc1375f/summary.md)
- [全量：178 / 178](../reports/acceptance/aicc-t1-2026-10-10T03-53-26-950Z-b9b6b849/summary.md)
- [全量：180 passed，4 failed](../reports/acceptance/aicc-t1-2026-10-10T14-41-34-375Z-34ab5e4e/summary.md)
- [全量：182 passed，2 failed](../reports/acceptance/aicc-t1-2026-10-10T14-50-19-784Z-21a637ea/summary.md)
- [全量：182 passed，2 failed](../reports/acceptance/aicc-t1-2026-10-10T15-01-38-653Z-4b7e6956/summary.md)
- [定向：0 passed，2 failed](../reports/acceptance/aicc-t1-2026-10-10T15-08-49-934Z-c64d01f5/summary.md)
- [定向：0 passed，2 failed](../reports/acceptance/aicc-t1-2026-10-10T15-11-10-593Z-fa0fd08a/summary.md)
- [定向：2 / 2](../reports/acceptance/aicc-t1-2026-10-10T15-18-20-257Z-b531e103/summary.md)
- [最终全量：184 / 184](../reports/acceptance/aicc-t1-2026-10-10T15-20-17-813Z-ad2e4135/summary.md)

### T1.5

- [Provider 全量：1577 passed，28 failed](../reports/acceptance/t15-20261010040037-2623351/summary.md)
- [定向：0 passed，1 failed](../reports/acceptance/t15-20261010043154-2665390/summary.md)
- [定向：0 passed，1 failed](../reports/acceptance/t15-20261010043220-2666006/summary.md)
- [定向：0 passed，1 failed](../reports/acceptance/t15-20261010043249-2666796/summary.md)
- [定向：1 / 1](../reports/acceptance/t15-20261010044035-2678347/summary.md)
- [Provider 全量：1605 / 1605](../reports/acceptance/t15-20261010044054-2678826/summary.md)
- [定向：0 passed，1 failed](../reports/acceptance/t15-20261010072125-2895355/summary.md)
- [Provider 全量：1605 / 1605](../reports/acceptance/t15-20261010072136-2895665/summary.md)
- [Provider 全量：1605 / 1605](../reports/acceptance/t15-20261010101158-3080107/summary.md)
- [DeepSeek：30 / 30](../reports/acceptance/t15-20261008102309-22405/summary.md)
- [Kimi 授权模型：26 / 26](../reports/acceptance/t15-20261008102337-22781/summary.md)
- [MiniMax：82 / 82](../reports/acceptance/t15-20261008104624-67760/summary.md)
- [Kimi 官方四模型：35 / 35](../reports/acceptance/t15-20261010152531-3539394/summary.md)
- [Kimi 推理内容回归：35 / 35](../reports/acceptance/t15-20261010160418-3615574/summary.md)

### T2

- [DeepSeek 与 Kimi / MiniMax 首轮目标矩阵](../reports/acceptance/aicc-2026-10-08T10-27-44-863Z-53cd8c14/summary.md)
- [DeepSeek Flash vision.caption 最终结果](../reports/acceptance/aicc-2026-10-08T14-04-29-790Z-ded511de/summary.md)
- [Kimi 旧模型 LLM 与 MiniMax 旧模型 LLM](../reports/acceptance/aicc-2026-10-08T05-44-41-155Z-e37ceeba/summary.md)
- [Kimi 旧模型 vision.caption](../reports/acceptance/aicc-2026-10-08T05-54-06-572Z-2d9bf678/summary.md)
- [Kimi 旧模型 vision.ocr](../reports/acceptance/aicc-2026-10-08T06-02-49-682Z-0dc74172/summary.md)
- [Kimi K3 与 K2.7 Code HighSpeed 六个单元](../reports/acceptance/aicc-2026-10-08T14-23-08-656Z-e504c9eb/summary.md)
- [MiniMax 旧模型 M3 视觉](../reports/acceptance/aicc-2026-10-08T06-04-51-424Z-fb8605b0/summary.md)
- [MiniMax 新增模型七个单元](../reports/acceptance/aicc-2026-10-08T10-49-33-585Z-a00bd81c/summary.md)
- [MiniMax 图生视频两个单元](../reports/acceptance/aicc-2026-10-08T10-53-03-815Z-e9a08ce1/summary.md)
- [Kimi 库存基线：0 passed，2 failed，8 skipped](../reports/acceptance/aicc-2026-10-10T05-11-54-561Z-6db0b95a/summary.md)
- [Kimi 零调用计划：6 skipped](../reports/acceptance/aicc-2026-10-10T05-19-26-458Z-0cdb4f23/summary.md)
- [Kimi 六单元：6 / 6](../reports/acceptance/aicc-2026-10-10T05-19-50-091Z-5a533960/summary.md)
- [Kimi 零调用计划：6 skipped](../reports/acceptance/aicc-2026-10-10T08-05-17-002Z-e8f703be/summary.md)
- [Kimi 零调用计划：6 skipped](../reports/acceptance/aicc-2026-10-10T10-43-26-306Z-a7a3aeaf/summary.md)
- [Kimi 官方库存校验：2 failed，6 skipped](../reports/acceptance/aicc-2026-10-10T15-27-09-864Z-9f789915/summary.md)
- [Kimi 官方四模型零调用计划：12 skipped](../reports/acceptance/aicc-2026-10-10T15-29-03-475Z-f7764b07/summary.md)
- [Kimi 官方四模型：3 passed，9 failed](../reports/acceptance/aicc-2026-10-10T15-29-19-561Z-710c1dec/summary.md)
- [Kimi 官方四模型最终结果：12 / 12](../reports/acceptance/aicc-2026-10-10T16-05-07-946Z-b2b047ea/summary.md)
