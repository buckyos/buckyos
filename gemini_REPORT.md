# AICC Gemini T1 / T1.5 / T2 测试报告

## 最终汇总

| 层级 | 最终结果 | 汇总 |
| --- | --- | --- |
| T1 | 通过 | 189 / 189 passed |
| T1.5 | 通过 | 最新全量 2510 / 2510 passed；更新后的 Gemini 414 / 414 passed |
| T2 | 未全通过 | 当前完整矩阵 36 个物理模型、123 个用例：16 passed、83 review、13 provider_restricted、11 failed |

T2 的 11 个 failed 中，7 个因 Gemini 拒绝抓取当前 NDN 私网/排除 IP URL，1 个同步请求返回网关 500，1 个后台 custom-tools agent 不接受图像输入，2 个 Veo extension 被 Provider 以 `durationSeconds` 越界拒绝。修复后的真实文本 Embedding 为 2 / 2 passed。当前 123 个 `<model, api-type>` 均已有非 skipped 的 T2 结果，矩阵预检无 capability baseline mismatch。

Google 当前 Thinking 文档仍声明 `gemini-3.8-flash` 与 `gemini-2.5-flash` 支持 `medium`。线上参数回归中，3.8 接受真实 `medium`；2.5 仍返回 HTTP 400 并仅允许 `high/low`，因此保留模型级 `reasoning-medium`，仅在 Google Gemini 渠道把其底层参数映射为 `high`。Lyria 用例补测将原先的 skipped 单元补齐为 `review`，当前矩阵总数仍为 123；真实生成、产物下载与完整性校验均成功，按要求未启用 Judge。

### `vision.detect` / `vision.segment` 专项汇总

| 层级 | API type | 最终结果 | 汇总 |
| --- | --- | --- | --- |
| T1.5 | `vision.detect` | 通过 | 最终部署版本 1 / 1 passed |
| T1.5 | `vision.segment` | 通过 | 最终部署版本 1 / 1 passed |
| T2 | `vision.detect` | 部分通过 | 14 个物理模型：12 review、2 provider_restricted |
| T2 | `vision.segment` | 未全通过 | 14 个物理模型：11 review、2 provider_restricted、1 failed |

专项 T2 共执行 28 次真实调用；23 个用例完成 Provider 协议调用并产生结构化结果，因本轮未启用 Judge 而标记为 `review`。`gemini-2.5-flash-lite` 与 `gemini-2.5-pro` 的检测、分割共 4 个用例被 Provider 明确限制为新用户不可用。`gemini-3.1-pro-preview-customtools` 分割在同步调用中超过 cyfs-gateway 的 60 秒响应头期限；该模型改用官方 `background` 交互时会路由到不支持图像输入的 `-agent` 变体，因此不能用后台模式规避超时。专项调用已知实际费用为 USD 0.00586245；另有 25 次调用缺少 Provider 结算数据，报告按上限记录 USD 0.25 的未知费用敞口。

## T1 详细报告

- [T1 全量：189 / 189](test/aicc_test/reports/acceptance/aicc-t1-2026-10-14T10-13-42-029Z-22fc0ff2/summary.md)
- [最新 T1 全量：189 / 189](test/aicc_test/reports/acceptance/aicc-t1-2026-10-16T07-42-46-007Z-845ce456/summary.md)
- [能力基线修正后 T1 全量：189 / 189](test/aicc_test/reports/acceptance/aicc-t1-2026-10-16T08-43-15-887Z-8631ec17/summary.md)

## T1.5 详细报告

- [t15-20261014102030-273359：781 / 786](test/aicc_test/reports/acceptance/t15-20261014102030-273359/summary.md)
- [t15-20261014103839-301910：4 / 4](test/aicc_test/reports/acceptance/t15-20261014103839-301910/summary.md)
- [t15-20261014103905-302443：1806 / 1806](test/aicc_test/reports/acceptance/t15-20261014103905-302443/summary.md)
- [t15-20261014112147-365077](test/aicc_test/reports/acceptance/t15-20261014112147-365077/summary.md)
- [t15-20261014112155-365386](test/aicc_test/reports/acceptance/t15-20261014112155-365386/summary.md)
- [t15-20261014112703-373321](test/aicc_test/reports/acceptance/t15-20261014112703-373321/summary.md)
- [t15-20261014113109-378574：399 / 399](test/aicc_test/reports/acceptance/t15-20261014113109-378574/summary.md)
- [t15-20261014113923-390256：405 / 405](test/aicc_test/reports/acceptance/t15-20261014113923-390256/summary.md)
- [t15-20261014120240-424758](test/aicc_test/reports/acceptance/t15-20261014120240-424758/summary.md)
- [t15-20261014120302-425312](test/aicc_test/reports/acceptance/t15-20261014120302-425312/summary.md)
- [t15-20261014120405-426797：405 / 405](test/aicc_test/reports/acceptance/t15-20261014120405-426797/summary.md)
- [t15-20261014122000-449375：4 / 4](test/aicc_test/reports/acceptance/t15-20261014122000-449375/summary.md)
- [t15-20261014122526-457901：1 / 1](test/aicc_test/reports/acceptance/t15-20261014122526-457901/summary.md)
- [t15-20261014131325-525898：73 / 76](test/aicc_test/reports/acceptance/t15-20261014131325-525898/summary.md)
- [t15-20261014131746-532919：2 / 2](test/aicc_test/reports/acceptance/t15-20261014131746-532919/summary.md)
- [t15-20261014131804-533426：414 / 414](test/aicc_test/reports/acceptance/t15-20261014131804-533426/summary.md)
- [t15-20261014140217-595207：检测/分割 2 / 2](test/aicc_test/reports/acceptance/t15-20261014140217-595207/summary.md)
- [t15-20261014142249-625300：检测/分割 2 / 2](test/aicc_test/reports/acceptance/t15-20261014142249-625300/summary.md)
- [t15-20261014144406-655193：最终部署版本检测/分割 2 / 2](test/aicc_test/reports/acceptance/t15-20261014144406-655193/summary.md)
- [最新 T1.5 全量：2510 / 2510](test/aicc_test/reports/acceptance/t15-20261016074720-4116059/summary.md)
- [能力基线修正后 T1.5 全量：2510 / 2510](test/aicc_test/reports/acceptance/t15-20261016084809-10062/summary.md)

## T2 详细报告

- [aicc-2026-10-14T11-10-39-661Z-90019e27](test/aicc_test/reports/acceptance/aicc-2026-10-14T11-10-39-661Z-90019e27/summary.md)
- [aicc-2026-10-14T11-35-43-776Z-4f156a50](test/aicc_test/reports/acceptance/aicc-2026-10-14T11-35-43-776Z-4f156a50/summary.md)
- [aicc-2026-10-14T11-35-54-582Z-c0699f43](test/aicc_test/reports/acceptance/aicc-2026-10-14T11-35-54-582Z-c0699f43/summary.md)
- [aicc-2026-10-14T11-43-48-711Z-439069a0](test/aicc_test/reports/acceptance/aicc-2026-10-14T11-43-48-711Z-439069a0/summary.md)
- [aicc-2026-10-14T11-44-33-991Z-61727f70：41 calls](test/aicc_test/reports/acceptance/aicc-2026-10-14T11-44-33-991Z-61727f70/summary.md)
- [aicc-2026-10-14T11-53-28-431Z-26d820b1](test/aicc_test/reports/acceptance/aicc-2026-10-14T11-53-28-431Z-26d820b1/summary.md)
- [aicc-2026-10-14T12-09-04-912Z-90defb00：16 cases](test/aicc_test/reports/acceptance/aicc-2026-10-14T12-09-04-912Z-90defb00/summary.md)
- [aicc-2026-10-14T12-20-20-754Z-4abb2686：3 cases](test/aicc_test/reports/acceptance/aicc-2026-10-14T12-20-20-754Z-4abb2686/summary.md)
- [aicc-2026-10-14T12-25-42-474Z-aca5cc84：1 case](test/aicc_test/reports/acceptance/aicc-2026-10-14T12-25-42-474Z-aca5cc84/summary.md)
- [aicc-2026-10-14T12-26-31-138Z-80676abe](test/aicc_test/reports/acceptance/aicc-2026-10-14T12-26-31-138Z-80676abe/summary.md)
- [aicc-2026-10-14T12-45-35-924Z-5bc023c3：27 cases](test/aicc_test/reports/acceptance/aicc-2026-10-14T12-45-35-924Z-5bc023c3/summary.md)
- [aicc-2026-10-14T12-51-12-164Z-d93a924c：30 cases](test/aicc_test/reports/acceptance/aicc-2026-10-14T12-51-12-164Z-d93a924c/summary.md)
- [aicc-2026-10-14T13-00-40-541Z-0d886257：25 cases](test/aicc_test/reports/acceptance/aicc-2026-10-14T13-00-40-541Z-0d886257/summary.md)
- [aicc-2026-10-14T13-22-38-279Z-4109a0b6：2 / 2](test/aicc_test/reports/acceptance/aicc-2026-10-14T13-22-38-279Z-4109a0b6/summary.md)
- [aicc-2026-10-14T13-24-21-673Z-de64fa9f：123 cases](test/aicc_test/reports/acceptance/aicc-2026-10-14T13-24-21-673Z-de64fa9f/summary.md)
- [aicc-2026-10-14T13-26-55-952Z-2ec28919：13 cases](test/aicc_test/reports/acceptance/aicc-2026-10-14T13-26-55-952Z-2ec28919/summary.md)
- [aicc-2026-10-14T14-03-14-352Z-54ec94b5：检测/分割 28 cases](test/aicc_test/reports/acceptance/aicc-2026-10-14T14-03-14-352Z-54ec94b5/summary.md)
- [aicc-2026-10-14T14-36-17-432Z-81486ca8：后台交互能力复核 1 case](test/aicc_test/reports/acceptance/aicc-2026-10-14T14-36-17-432Z-81486ca8/summary.md)
- [medium 变体线上回归：2 个指定模型](test/aicc_test/reports/acceptance/gemini-medium-regression-20261016/summary.md)
- [新增 `lyria-3.5 + audio.music`：真实调用与产物校验完成，未启用 Judge](test/aicc_test/reports/acceptance/aicc-2026-10-16T08-31-29-380Z-efecfea8/summary.md)
- [最终矩阵预检：123 个组合，0 baseline mismatch](test/aicc_test/reports/acceptance/aicc-2026-10-16T09-31-42-766Z-38912391/summary.md)
