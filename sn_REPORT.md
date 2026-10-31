# SN AICC 汇总测试报告

更新日期：2026-10-31（北京时间）；轮次 ID 保留原 UTC 时间。BuckyOS 基准提交 `d40291a9a`；SN 配置基准提交 `c23a55b4`。真实测试仅使用授权的 SN GLOBAL_OPEN、US_FULL 两个实例。

## 后续诊断的当前结论

公共 SQLite/执行器修复已部署，运行数据库为 WAL。[修复后定向复测](test/aicc_test/sn_reports/20261030/local-fix-retest/summary.md)：T1.5 30/30 通过后，T2 三个目标任务全部成功进入终态。原始 T2 内容状态为 review=3；其中图编辑人工比较通过，两个 Pro OCR 保留 review。此前 21 项识别质量结果不变。

NDN 下载问题已解决，不再列为当前待修缺陷。NDM Proxy 405 不代表 NDN 不支持下载：现有 cyfs-dir 加配置即可读取制品，[34 个既有对象回读](test/aicc_test/sn_reports/20261030/ndn-readback/summary.md)全部成功。本轮也使用原生 NDN 完成前置图与最终图读取。删除接口仍缺失，本轮两个对象清理失败单独记录。

SN 缺陷已提交并核验：[Qwen Image/Wan 补充 #49](https://github.com/buckyos/sn-business/issues/49#issuecomment-6527012935)、[Terra #53](https://github.com/buckyos/sn-business/issues/53)。[逐项诊断](test/aicc_test/sn_reports/20261030/followup-diagnosis/summary.md)保留技术依据。Grok 没有证据表明本次因大陆用户而拒绝；不将模型拒答等同于地区封禁。

## 最终汇总

T1.5 门禁通过后执行 T2；以下统计按各轮并集合并，重复单元取最后一次结果。人工复核与自动结果分别保留，原始轮次报告未改写。

| 范围 | 用例数 | 通过 | 失败 | 待复核 |
|---|---:|---:|---:|---:|
| SN T1.5（去重） | 1219 | 1219 | 0 | 0 |
| 受影响原厂/代理通道 T1.5 | 102 | 102 | 0 | 0 |
| SN T2 GLOBAL_OPEN | 42 | 31 | 11 | 0 |
| SN T2 US_FULL | 124 | 69 | 53 | 2 |
| SN T2 合计 | 166 | 100 | 64 | 2 |

下表统计实际执行与复核记录，和当前缺陷是否关闭分开管理：NDN 旧失败记录保留，但下载问题已经关闭。T2 仍有失败及待复核内容，整体未通过。修复后当前部署的定向 T1.5 为 30 项，表中 1219/102 是跨轮历史覆盖，不是本轮全部重跑。T1.5 的 102 项关联覆盖为 OpenAI 12、GLM 24、OpenRouter 51、Kimi 12、Qwen 3；这些非 SN 通道未执行真实 T2，已有对应模型/API 的矩阵配置。

真实模型调用共 **176 次**（包括生成前置资源的调用）；无自动重试。各轮成本字段均不能确认最终结算金额，合计未知成本估算暴露 **$3.68**，不是实际账单，也不代表免费；各轮均未触发 runner 的估算预算检查，不构成实际结算上限保证。

当前待处理的是 SN 上游图片/视频转发协议、Terra background/computer 闭环及 Grok 拒答根因；21 项内容质量结果保留，不继续追查。NDN 下载已验证可用；NDM Proxy 删除接口缺失仅作为独立清理限制记录。定位依据及收尾状态见[边界与失败证据](test/aicc_test/sn_reports/20261030/external-validation/summary.md)。

[内容复核明细](test/aicc_test/sn_reports/20261030/manual-review/summary.md)记录 69 次复核、48 次通过、21 次失败。OCR 按用例提示要求核对完整 `BUCKYOS-DV-4827`，比自动 Judge 的仅检查数字 `4827` 更严格；caption 核对显著图形和数量，ASR 对照官方 JFK 录音。复核没有新增模型调用。

Realtime 与 UI-TARS 原生 computer-use 不计入可调用矩阵：当前适配器分别缺少对应 WebSocket 传输和原生工具协议支持。逻辑别名/推理预设在 T1.5 覆盖，T2 仅测试基础物理模型。

验证：AICC Rust 单测 789/789；acceptance 自测 150/150；相关 Deno 类型检查、AICC release 构建、`git diff --check` 通过。未运行完整 workspace `cargo test`、完整 `buckyos-build.py` 或其他原厂 Provider 真实调用。Zone 已激活，服务及主要端口在线；健康检查另提示 repo-service 错误日志较多。

## 禁用项、新增模型及失败分类

[分项汇总](test/aicc_test/sn_reports/20261030/scope-summary/summary.md)列出原禁用项、当前开放状态，以及新增 25 个 SN 条目的模型/API 结论。

- 恢复的 9 个 API 单元均通过 T1.5，但最终均未通过 T2；失败后未恢复禁用，当前仍开放。Realtime 与 UI-TARS 原生 computer-use 仍禁用，Sora 条目已移除。
- 新增条目中，10 个在本次 T2 所选 API 上全部通过，7 个部分通过，2 个被制品读取阻断；5 个别名/预设仅独立执行 T1.5，1 个 Realtime 条目保持禁用。
- 合并执行记录仍有 64 个失败，其中 33 个是旧 NDM Proxy 读取阻塞记录；下载问题现已解决，这 33 项不再代表当前 NDN 缺陷，未重跑的模型单元也不改写为新通过。其余为 Qwen Image 上游路由 2、Wan 上游 Content-Type 3、Terra 协议响应 4、Grok 拒答 1、内容复核 21。两个 Pro OCR 的运行故障复测成功，内容保留 review；GPT-5.5 img2img 已通过本轮人工比较。

## 每轮详细报告

下列链接覆盖本次所有生成报告的执行轮次，包括部分范围、历史失败和无真实调用的矩阵检查；不能将任意单轮视作全部覆盖。

| 层次 | 轮次报告 | 本轮结果 |
|---|---|---|
| T2 矩阵检查 | [aicc-2026-10-30T08-48-26-328Z-e63f03c2](test/aicc_test/sn_reports/20261030/aicc-2026-10-30T08-48-26-328Z-e63f03c2/summary.md) | skipped=42 |
| T2 矩阵检查 | [aicc-2026-10-30T08-50-01-373Z-7254bd5b](test/aicc_test/sn_reports/20261030/aicc-2026-10-30T08-50-01-373Z-7254bd5b/summary.md) | skipped=124 |
| T2 | [aicc-2026-10-30T08-56-04-646Z-9e09dbd4](test/aicc_test/sn_reports/20261030/aicc-2026-10-30T08-56-04-646Z-9e09dbd4/summary.md) | failed=7，passed=17，review=18 |
| T2 | [aicc-2026-10-30T09-01-10-151Z-69abbe3b](test/aicc_test/sn_reports/20261030/aicc-2026-10-30T09-01-10-151Z-69abbe3b/summary.md) | failed=43，passed=34，review=48 |
| T2 | [aicc-2026-10-30T09-21-18-381Z-cb64369b](test/aicc_test/sn_reports/20261030/aicc-2026-10-30T09-21-18-381Z-cb64369b/summary.md) | failed=5，review=2 |
| T2 | [aicc-2026-10-30T09-26-46-334Z-41139d0e](test/aicc_test/sn_reports/20261030/aicc-2026-10-30T09-26-46-334Z-41139d0e/summary.md) | review=1 |
| T1.5 | [t15-20261030072132-891931](test/aicc_test/sn_reports/20261030/t15-20261030072132-891931/summary.md) | passed=464，skipped=70 |
| T1.5 | [t15-20261030074133-915081](test/aicc_test/sn_reports/20261030/t15-20261030074133-915081/summary.md) | failed=1 |
| T1.5 | [t15-20261030074247-918052](test/aicc_test/sn_reports/20261030/t15-20261030074247-918052/summary.md) | failed=1 |
| T1.5 | [t15-20261030074554-921564](test/aicc_test/sn_reports/20261030/t15-20261030074554-921564/summary.md) | failed=9，passed=115 |
| T1.5 | [t15-20261030075947-956221](test/aicc_test/sn_reports/20261030/t15-20261030075947-956221/summary.md) | failed=6，passed=118 |
| T1.5 | [t15-20261030080314-960072](test/aicc_test/sn_reports/20261030/t15-20261030080314-960072/summary.md) | failed=66，passed=661 |
| T1.5 | [t15-20261030081911-983492](test/aicc_test/sn_reports/20261030/t15-20261030081911-983492/summary.md) | passed=855 |
| T1.5 | [t15-20261030082929-994734](test/aicc_test/sn_reports/20261030/t15-20261030082929-994734/summary.md) | passed=172 |
| T1.5 | [t15-20261030083346-999766](test/aicc_test/sn_reports/20261030/t15-20261030083346-999766/summary.md) | passed=12 |
| T1.5 | [t15-20261030083353-999979](test/aicc_test/sn_reports/20261030/t15-20261030083353-999979/summary.md) | failed=1 |
| T1.5 | [t15-20261030083704-1003203](test/aicc_test/sn_reports/20261030/t15-20261030083704-1003203/summary.md) | passed=24 |
| T1.5 | [t15-20261030083714-1003438](test/aicc_test/sn_reports/20261030/t15-20261030083714-1003438/summary.md) | failed=1 |
| T1.5 | [t15-20261030083934-1005894](test/aicc_test/sn_reports/20261030/t15-20261030083934-1005894/summary.md) | failed=1 |
| T1.5 | [t15-20261030084217-1008932](test/aicc_test/sn_reports/20261030/t15-20261030084217-1008932/summary.md) | passed=51 |
| T1.5 | [t15-20261030084233-1009223](test/aicc_test/sn_reports/20261030/t15-20261030084233-1009223/summary.md) | passed=12 |
| T1.5 | [t15-20261030084240-1009446](test/aicc_test/sn_reports/20261030/t15-20261030084240-1009446/summary.md) | passed=3 |
| T1.5 | [t15-20261030084415-1011228](test/aicc_test/sn_reports/20261030/t15-20261030084415-1011228/summary.md) | passed=192 |
| T1.5 | [t15-20261030085437-1022247](test/aicc_test/sn_reports/20261030/t15-20261030085437-1022247/summary.md) | passed=2 |
| T1.5 | [t15-20261030092029-1050728](test/aicc_test/sn_reports/20261030/t15-20261030092029-1050728/summary.md) | passed=16 |
| T1.5 | [t15-20261030155506-1454156](test/aicc_test/sn_reports/20261030/t15-20261030155506-1454156/summary.md) | passed=30 |
| T2 | [aicc-2026-10-30T15-56-30-584Z-312e474a](test/aicc_test/sn_reports/20261030/aicc-2026-10-30T15-56-30-584Z-312e474a/summary.md) | review=3，cleanup failed=1；运行链路 3/3 成功，图编辑人工通过 |

每个轮次只保留可提交的 `summary.md`、`finance.md`、`effective_baseline.json` 概要。逐 case JSON、原始响应及制品仍位于本机被忽略的 `test/aicc_test/reports/acceptance/<run-id>/`。
