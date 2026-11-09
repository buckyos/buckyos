# SN AICC 汇总测试报告

更新日期：2026-11-09（北京时间）；轮次 ID 保留原 UTC 时间。历史 BuckyOS 基准提交 `d40291a9a`，PR 基于上游 `803d0ee87`，包含后续工作区更新；SN 历史配置基准提交 `c23a55b4`；2026-11-09 服务端升级说明见 #53，当前部署提交未独立核验。真实测试仅使用授权的 SN GLOBAL_OPEN、US_FULL 两个实例。

PR 推送前验证见[本轮汇总及各轮完整报告](test/aicc_test/sn_reports/20261109/pr-verification/summary.md)：路由 T1 91/91；SN T1.5 1149 通过、70 策略跳过；相关渠道定向 T1.5 102/102。扩大范围检查另有 4 个 OpenAI 视频错误码断言失败和 1 个 NDM Proxy 405 中断，未列为通过。无新真实模型调用。

## 当前 SN 渠道策略与验收口径

[2026-11-09 渠道策略汇总](test/aicc_test/sn_reports/20261109/channel-policy/summary.md)链接配置结论、Grok 验收依据、运行时库存及本轮完整测试报告。Grok 4.7 LLM 按用户决定记为**通过（用户接受）**，原始拒答和自动失败记录保留，本轮没有重新调用或放宽标记断言。#54 的服务端证据已将该次拒答定位到 OpenRouter 返回的 xAI 响应。

Qwen Image 3 Pro 的两个图片 API、Wan 3.0 的三个视频 API 均无成功项，且 #49 上游协议问题仍未解决，因此在 **sn.provider.json 整模型 exclude**。原厂能力及其他 Provider 保留；Qwen OCR、已恢复的 Terra、Grok 继续开放。已安装本地 SN metadata revision 15 并 reload，临时 SN 库存核验确认两个模型不可见、三个保留模型及其能力可见。

本轮无真实 T2 调用：SN 单测 12/12、acceptance 自测 152/152、Deno 类型检查通过；T1.5 **81 通过、54 按禁用策略跳过、0 失败**。跳过不算通过。临时设置和 Mock 实例清理通过，没有构建或重启 Zone。当前汇总扣除已禁用的 5 个历史 T2 单元；历史 172 个单元按用户接受后的口径为 114 通过、56 失败、2 待复核，其中 5 个失败现已禁用。

## Terra 服务端升级后的当前结论

[2026-11-09 复测合并报告](test/aicc_test/sn_reports/20261109/terra-retest/summary.md)链接本轮全部详细用例、账本和人工复核。根据 [#53 修复说明](https://github.com/buckyos/sn-business/issues/53#issuecomment-6734963329)，Terra 改为直接调用 OpenAI。T1.5 **56/56 通过**后，US_FULL 的四个历史失败 T2 单元 **全部通过**：computer-use 返回按钮范围内的左键点击；OCR 正确返回 `BUCKYOS-DV-4827`；txt2img 生成蓝方块和完整 4827；img2img 的前置与目标均成功，保留构图并加入暖色光照。原始自动状态 passed=1、review=3，人工复核后 4/4 通过。

runner 推理调用 5 次（4 目标 + 1 前置）；computer-use 的一次调用记录 2 个上游 request_units，按本轮记录计 6 次上游交互，无自动重试。估算敞口 $0.12 / 预算 $2，最终结算未知。图片均成功读取；临时设置、凭据和 NDN 路由恢复。3 个生成对象的删除仍返回 HTTP 405，清理失败单列。客户端提示、异步操作和验收条件保持原样，没有本地产品修改、构建或部署。

本轮验证了 #53 对应四项历史故障已在 US_FULL 恢复；原始失败轮次保留，以下合并矩阵用最新结果替换这四项。其他模型及既有内容质量结果未重跑。

## 新增模型参数复核

本轮对 5 个新增原厂模型重新核对官方技术参数，并同步 SN/Kimi/OpenRouter 的固定参数规则及测试基线。[本轮合并报告](test/aicc_test/sn_reports/20261102/model-parameter-review/summary.md)链接全部执行轮次及各轮详细用例；最终有效 T1.5 为 **162/162 通过**（SN 63、OpenAI 12、Kimi 12、GLM 24、OpenRouter 51）。相关 Rust 检查 53/53、acceptance 自测 151/151、Deno 类型检查及 Gateway 收尾检查通过。该参数复核轮未新增真实 T2；下节图片重测已更新最终 T2 汇总。

这 5 个模型并非全部完成真实 T2 验收：Flare/Sunburst 已完成三种 API 的目标调用和读取，内容验收各 2 通过、1 未通过；Kimi OCR 保留质量失败；FlashX 已独立执行两个账号的 6 个 T2 单元，5 通过、1 OCR 内容未通过；Grok LLM 已按用户验收决定接受为通过。原厂参数与能力覆盖边界见[官方参数复核表](doc/aicc/driver_metadata_review_20261102.md)。

## 图片 T2 重测与 FlashX 身份

[图片重测合并报告](test/aicc_test/sn_reports/20261102/image-retest/summary.md)链接各轮完整用例、调用账本、NDN 回读和内容复核。T1.5 12/12 通过后，Flare、Sunburst 各 txt2img/img2img/inpaint 共 6 个目标全部执行成功、结果均可下载；完整内容验收 **4 通过、2 未通过**。两个 inpaint 都生成了中央绿叶，但没有保留蓝色背景，实际为透明区域。共 8 次真实调用（含 2 个前置生成），无自动重试；8 个任务全部为 Terminal/Succeeded，8 个输出最终 NDN 回读和 PNG 校验通过。

NDN 临时路由的旧 HTTP 长连接曾返回 403；同一对象用新连接或独立 localhost origin 均返回 200。已用本地临时配置隔离 NDN/RPC 连接池，未改 Gateway/NDN 实现。Provider 设置和临时路由已恢复；8 个输出对象删除仍返回 405，清理失败单列，不算模型推理失败。

FlashX 的实际映射始终为 FlashX：AICC `glm/glm-5.3-flashx`，SN 转发 `openrouter/z-ai/glm-5.3-flashx`。已删除依据不足的 T2 FlashX→Flash 去重规则；原厂独立 API 代码保留独立验收单元。GLOBAL_OPEN/US_FULL 各 3 个 FlashX API 单元已完成独立 T2，计入下表 172 个已执行单元；整体 T2 仍有失败与待复核内容。相关自测 152/152 和类型检查通过。

## FlashX 独立 T2 结果

[FlashX 合并报告](test/aicc_test/sn_reports/20261102/flashx-t2/summary.md)链接本轮 T1.5、两个账号的 T2 完整用例及内容复核。T1.5 12/12 通过；真实 T2 执行 6 次、无重试，GLOBAL_OPEN 为 2 通过/1 OCR 内容未通过，US_FULL 为 3 通过。六个任务均为 Terminal/Succeeded，没有协议失败。GLOBAL_OPEN OCR 预期 `BUCKYOS-DV-4827`，实际 `BUCKY05-DV-4827`；US_FULL 返回正确标记。两个账号 LLM 和 caption 都通过。两个执行轮次的临时设置和资源 cleanup 均通过。

本轮估算敞口 $0.12，合计估算预算 $1；最终结算金额未知。两个图片模型没有再次调用，它们的剩余问题是 inpaint 背景保留未达要求，不能概括为所有图片画质不够。

## Terra 2026-11-02 历史失败 T2 重测

[本轮合并报告](test/aicc_test/sn_reports/20261102/terra-retest/summary.md)链接 T1.5、T2 各轮详细用例和逐任务 trace。T1.5 **56/56 通过**后，仅重测 US_FULL 的 Terra computer-use、txt2img、img2img、OCR 四个历史失败单元，**均未通过**：computer-use 仍没有 computer action；OCR、txt2img 和 img2img 的生成前置仍未确认接受 Responses background 执行，img2img 目标没有发起。不是轮询超时，也没有进入 NDN 下载。

实际 4 次调用、无重试，估算敞口 $0.10 / 预算 $2，最终结算未知；临时配置、路由与资源 cleanup passed。没有修改产品实现、模型参数或渠道操作，没有构建部署。该历史轮仍复现 [SN #53](https://github.com/buckyos/sn-business/issues/53)；最新 2026-11-09 复测已通过，合并状态及证据以上方升级后报告为准。

## 后续诊断的当前结论

公共 SQLite/执行器修复已部署，运行数据库为 WAL。[修复后定向复测](test/aicc_test/sn_reports/20261030/local-fix-retest/summary.md)：T1.5 30/30 通过后，T2 三个目标任务全部成功进入终态。原始 T2 内容状态为 review=3；其中图编辑人工比较通过，两个 Pro OCR 保留 review。此前 21 项识别质量结果不变。

NDN 下载问题已解决，不再列为当前待修缺陷。NDM Proxy 405 不代表 NDN 不支持下载：现有 cyfs-dir 加配置即可读取制品，[34 个既有对象回读](test/aicc_test/sn_reports/20261030/ndn-readback/summary.md)全部成功。本轮也使用原生 NDN 完成前置图与最终图读取。删除接口仍缺失，该历史轮两个对象清理失败单独记录；本次图片重测另有 8 个输出对象删除失败。

SN 缺陷已提交并核验：[Qwen Image/Wan 补充 #49](https://github.com/buckyos/sn-business/issues/49#issuecomment-6527012935)、[Terra #53](https://github.com/buckyos/sn-business/issues/53)。[逐项诊断](test/aicc_test/sn_reports/20261030/followup-diagnosis/summary.md)保留技术依据。Grok 没有证据表明本次因大陆用户而拒绝；不将模型拒答等同于地区封禁。 Grok 拒答已另行提交 [#54](https://github.com/buckyos/sn-business/issues/54)；[服务端核查](https://github.com/buckyos/sn-business/issues/54#issuecomment-6569762068)确认该次拒答已存在于 OpenRouter 返回的 xAI 响应中，现按用户决定接受通过；[最终提交内容](test/aicc_test/sn_reports/20261102/grok-issue-draft/summary.md)保留原始请求与关联标识。

## 最终汇总

T1.5 门禁通过后执行 T2；以下 T2 统计是已有历史矩阵扣除当前已禁用 5 个单元后的范围，重复单元取最新结果。通过数含 Grok 用户接受 1 项；人工复核、用户验收与原始自动结果分别保留，原始轮次报告未改写。

| 范围 | 用例数 | 通过 | 失败 | 待复核 |
|---|---:|---:|---:|---:|
| SN T1.5（去重） | 1219 | 1219 | 0 | 0 |
| 受影响原厂/代理通道 T1.5 | 102 | 102 | 0 | 0 |
| SN T2 GLOBAL_OPEN（启用范围） | 40 | 33 | 7 | 0 |
| SN T2 US_FULL（启用范围） | 127 | 81 | 44 | 2 |
| SN T2 合计（启用范围） | 167 | 114 | 51 | 2 |

上表统计实际执行与复核记录，和当前缺陷是否关闭分开管理：NDN 旧失败记录保留，但下载问题已经关闭。T2 仍有失败及待复核内容，整体未通过。历史公共组件修复后定向 T1.5 为 30 项；本次图片重测门禁为 12 项，表中 1219/102 是跨轮历史覆盖，不是本轮全部重跑。T1.5 的 102 项关联覆盖为 OpenAI 12、GLM 24、OpenRouter 51、Kimi 12、Qwen 3；这些非 SN 通道未执行真实 T2，已有对应模型/API 的矩阵配置。

runner 记录的真实推理调用累计 **199 次**（包括生成前置资源；本轮 computer-use 的一次调用含两个上游 request_units）；无自动重试。各轮成本字段均不能确认最终结算金额，合计未知成本估算暴露 **$4.18**，不是实际账单，也不代表免费；各轮均未触发 runner 的估算预算检查，不构成实际结算上限保证。

当前 SN 上游图片/视频协议缺口已按模型禁用；Grok 已用户接受；Terra background/computer 四项历史失败已于 2026-11-09 复测通过；既有 21 项内容质量结果保留，不继续追查；本次另记录 2 项 inpaint 背景保留未通过、1 项 FlashX OCR 识别内容错误。NDN 下载已验证可用；NDM Proxy 删除接口缺失仅作为独立清理限制记录。定位依据及收尾状态见[边界与失败证据](test/aicc_test/sn_reports/20261030/external-validation/summary.md)。

[内容复核明细](test/aicc_test/sn_reports/20261030/manual-review/summary.md)记录 69 次复核、48 次通过、21 次失败。OCR 按用例提示要求核对完整 `BUCKYOS-DV-4827`，比自动 Judge 的仅检查数字 `4827` 更严格；caption 核对显著图形和数量，ASR 对照官方 JFK 录音。复核没有新增模型调用。

Realtime 与 UI-TARS 原生 computer-use 不计入可调用矩阵：当前适配器分别缺少对应 WebSocket 传输和原生工具协议支持。逻辑别名/推理预设在 T1.5 覆盖，T2 仅测试基础物理模型。

验证：rebase 后 AICC Rust 单测 783/783；历史验收轮 AICC Rust 单测 789/789；历史 acceptance 自测 150/150，本轮 152/152；相关 Deno 类型检查、AICC release 构建、`git diff --check` 通过。未运行完整 workspace `cargo test`、完整 `buckyos-build.py` 或其他原厂 Provider 真实调用。Zone 已激活，服务及主要端口在线；健康检查另提示 repo-service 错误日志较多。

## 禁用项、新增模型及失败分类

[分项汇总](test/aicc_test/sn_reports/20261030/scope-summary/summary.md)列出原禁用项、当前开放状态，以及新增 25 个 SN 条目的模型/API 结论。

- 历史恢复的 9 个 API 单元中，Qwen Image 的 2 个、Wan 的 3 个已随整模型重新禁用；其余 4 个 OCR/inpaint 单元继续开放并保留原质量/读取结果。Realtime 与 UI-TARS 原生 computer-use 仍禁用，Sora 条目已移除。
- 新增条目的历史统计见分项报告；最新图片和 FlashX 的完整结果见上述 2026-11-02 合并报告。FlashX 已撤销别名去重并独立测试；Flare/Sunburst 的读取阻断已消除，各保留 1 项 inpaint 内容未通过。Realtime 条目保持禁用。
- 当前启用历史范围有 51 个失败：27 个尚未重跑的旧 NDM Proxy 读取阻塞记录、24 项内容验收未通过（既有 21 + 图片 inpaint 2 + FlashX OCR 1）。Qwen Image 2 项和 Wan 3 项失败保留在原轮次报告，已从当前启用范围扣除；Grok 按用户验收接受。下载能力已验证可用，未重跑的旧读取记录不改写为新通过。两个 Pro OCR 的运行故障复测成功，内容保留 review；GPT-5.5 img2img 已通过本轮人工比较。

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
