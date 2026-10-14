# AICC Fal T1 / T1.5 / T2 测试报告

## 最终汇总

| 层级 | 最终结果 | 覆盖与结果 |
|---|---|---|
| T1 | 通过 | 189/189 passed；manifest 130/130、requirement branches 127/127、route cases 59/59 |
| T1.5 全量 | 通过 | 15 个 Provider，共 1806/1806 passed |
| T1.5 Fal 回归 | 通过 | Fal 56/56 passed，0 次真实 Provider 调用 |
| T2 Fal | 完成，无失败项 | 官方目录中的 4 个 endpoint 全部完成真实调用和产物验证；0 failed、4 review、0 confirmed product defects |

T2 最终覆盖 `fal-ai/deepfilternet3`、`fal-ai/esrgan`、`fal-ai/imageutils/rembg`、`fal-ai/video-upscaler`。四个结果均已通过任务终态、协议映射、产物下载、格式和结构断言；由于未配置独立 Judge，媒体语义结论保留为 `review`。最终轮调用 4/4，已知实际费用为 USD 0，未知费用估算敞口为 USD 0.04，未超过 USD 0.10 预算，清理通过。

## 各轮详细报告

| 层级 | 轮次结果 | 详细报告 |
|---|---|---|
| T1 | 123 passed / 66 failed | [aicc-t1-2026-10-14T08-05-07-021Z-f9771671](reports/acceptance/aicc-t1-2026-10-14T08-05-07-021Z-f9771671/summary.md) |
| T1 | 189/189 passed | [aicc-t1-2026-10-14T08-13-45-159Z-a84768dd](reports/acceptance/aicc-t1-2026-10-14T08-13-45-159Z-a84768dd/summary.md) |
| T1.5 全量 | 1806/1806 passed | [t15-20261014082041-59574](reports/acceptance/t15-20261014082041-59574/summary.md) |
| T2 Fal 预检 | 4 skipped，0 次真实调用 | [aicc-2026-10-14T09-14-41-414Z-c02b6f64](reports/acceptance/aicc-2026-10-14T09-14-41-414Z-c02b6f64/summary.md) |
| T2 Fal | 2 review / 2 failed | [aicc-2026-10-14T09-15-37-379Z-837bbe35](reports/acceptance/aicc-2026-10-14T09-15-37-379Z-837bbe35/summary.md) |
| T1.5 Fal | runner failed，0/56 executed | [t15-20261014093121-178275](reports/acceptance/t15-20261014093121-178275/summary.md) |
| T1.5 Fal | runner failed，0/56 executed | [t15-20261014093140-178777](reports/acceptance/t15-20261014093140-178777/summary.md) |
| T1.5 Fal | 56/56 passed | [t15-20261014093215-179644](reports/acceptance/t15-20261014093215-179644/summary.md) |
| T2 Fal 最终 | 4 review / 0 failed | [aicc-2026-10-14T09-34-38-475Z-de153605](reports/acceptance/aicc-2026-10-14T09-34-38-475Z-de153605/summary.md) |
