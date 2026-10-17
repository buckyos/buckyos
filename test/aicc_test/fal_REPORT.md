# AICC Fal T1 / T1.5 / T2 测试报告

## 最终汇总

| 层级 | 最终结果 | 覆盖与结果 |
|---|---|---|
| T1 | 通过 | 184/184 passed；manifest 130/130、requirement branches 127/127、route cases 54/54；Fal 4 个路由暴露项全部通过 |
| T1.5 Fal | 通过 | 56/56 passed；0 次真实 Provider 调用 |
| T2 Fal | 完成，无失败项 | 官方目录中的 4 个 endpoint 全部完成真实调用和产物验证；0 failed、4 review、0 confirmed product defects |

T2 覆盖 `fal-ai/imageutils/rembg`、`fal-ai/deepfilternet3`、`fal-ai/esrgan`、`fal-ai/video-upscaler`。四个结果均通过任务终态、协议映射、产物下载、格式和结构断言；由于未配置独立 Judge，媒体语义结论保留为 `review`。真实调用 4/4，已知实际费用为 USD 0，未知费用估算敞口为 USD 0.04，未超过合计 USD 0.04 的分轮预算，清理通过。

## 各轮详细报告

| 层级 | 轮次结果 | 详细报告 |
|---|---|---|
| T1 | 184/184 passed | [aicc-t1-2026-10-17T08-46-28-988Z-abf2078d](reports/acceptance/aicc-t1-2026-10-17T08-46-28-988Z-abf2078d/summary.md) |
| T1.5 Fal | 56/56 passed | [t15-20261017085109-1780169](reports/acceptance/t15-20261017085109-1780169/summary.md) |
| T2 Fal 预检（零预算） | 4 skipped，0 次真实调用 | [aicc-2026-10-17T08-53-17-246Z-101b1e32](reports/acceptance/aicc-2026-10-17T08-53-17-246Z-101b1e32/summary.md) |
| T2 Fal 预检 | 4 skipped，0 次真实调用 | [aicc-2026-10-17T08-53-28-035Z-83182a97](reports/acceptance/aicc-2026-10-17T08-53-28-035Z-83182a97/summary.md) |
| T2 Fal 免费项 | 1 review / 0 failed | [aicc-2026-10-17T08-53-57-613Z-e0db2642](reports/acceptance/aicc-2026-10-17T08-53-57-613Z-e0db2642/summary.md) |
| T2 Fal 收费项 | 3 review / 0 failed | [aicc-2026-10-17T08-55-29-475Z-d0d0fee0](reports/acceptance/aicc-2026-10-17T08-55-29-475Z-d0d0fee0/summary.md) |
