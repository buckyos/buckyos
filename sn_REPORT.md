# AICC SN Provider 验收报告

## 最终汇总

2026-10-19，Ubuntu。当前 SN 账户目录返回 25 个 alias，其中 22 个可执行模型构成 80 个模型/API 单元。按同一 case 的最新有效结果及人工复核合并为 **75 passed、1 failed、4 review**，尚未全量通过。

本次只定向复测原失败的 **23 个单元：22 passed、1 failed、0 review、0 skipped**，没有扩大模型或 Provider 的真实调用范围。其他 57 个单元沿用既有有效结果，不能视为新版本重新执行了全量 T2。

| API | passed | failed | review |
|---|---:|---:|---:|
| llm | 16 | 0 | 0 |
| embedding.text | 2 | 0 | 0 |
| audio.asr | 2 | 0 | 0 |
| audio.tts | 0 | 0 | 1 |
| vision.caption | 14 | 0 | 0 |
| vision.ocr | 13 | 1 | 0 |
| image.txt2img | 13 | 0 | 0 |
| image.img2img | 8 | 0 | 3 |
| image.inpaint | 1 | 0 | 0 |
| agent.computer_use | 6 | 0 | 0 |
| 合计 | 75 | 1 | 4 |

- 唯一失败为 `gpt-5.4-nano/vision.ocr`：任务及异步协议成功，但实际识别文本未包含 `4827`，属于内容正确性失败，不据此删除视觉能力或放宽用例标准。其他 3 个 OCR 复测满足原定数字 `4827` 的最小正确性 rubric，不代表整行文字逐字正确。
- 本次 5 个文生图、8 个图生图、6 个 computer-use 单元均通过。生成图片已人工查看，图生图已逐一对照源图确认构图保留及暖色傍晚光照。
- 4 个既有 review 不在本次扩展范围：`gpt-4o-mini-tts/audio.tts` 尚缺听审；`gpt-5.6-luna`、`gpt-6-astra`、`gpt-image-2` 的 `image.img2img` 尚缺语义复核。
- T1 全量 **189/189**；受影响 Provider 全量 T1.5 **865/865（SN 472、OpenAI 393）**，均为零真实模型调用。AICC Rust lib 单测 **643/643**，验收基础设施自测 **128/128**，TypeScript 检查通过。
- 本次接受 **31 个唯一 AICC 任务（23 个目标、8 个源图）**，全部 Terminal/Succeeded。六个 computer-use 任务各包含两次上游请求；合计 37 次成功推理请求的未知费用估算为 **$0.37**，SN 实际 USD 扣费未知，该估算不是账单或实际费用上限。轮询、下载及幂等回读不计为新推理。
- 本次 21 个生成输出对象已清理，临时 SN 设置及运行时实例已移除，既有 Provider 设置保留，无本次非终态任务。历史轮次的调用、费用及清理信息保留在各轮报告，不将 RPC 重放简单相加为实际推理费用。
- SN Responses 后台提交、任务查询和结果读取实测可用；本次没有因异步缺口新增 skip，也没有同步替代。动态登录 `provider.validate` 实测 `auth_valid=true`，发现 22 个可执行模型。

配置与覆盖边界：

- SN 发布目录的 33 个 alias 已纳入配置，29 个可执行，4 个显式排除：`gpt-realtime-2.1` 缺少公开 Realtime 路由及对应 canonical API；`sora-2`、`sora-2-pro`、`wan-3.0` 的 SN 视频任务缺少完整状态查询和结果闭环，均不发起真实调用。
- 当前账户未返回 `qwen3.6-plus`、`qwen3.7-plus`、`qwen3.8-max-0902`、`qwen3-embedding-8b`、`qwen-image-3-pro`、`ui-tars-1.5-7b`、`voxtral-small-24b-2507`；它们不计为本轮已测。`wan-3.0` 也未返回。
- 本次未修改原厂 `*.model.json` 参数；SN operation 选择位于 `sn.provider.json`。共享实现影响已由 T1 和 SN/OpenAI T1.5 覆盖，未扩大其他 Provider 的真实 T2。
- 既有 Qwen Image 3：T1 3/3、全量 T1.5 275/275，T2 文生图及图生图人工复核 2/2 通过，含源图共 3 次真实调用，未知费用估算 $0.03。本次未重新调用 Qwen。

## 本轮 T1 报告

1. [2026-10-19 T1 全量：189/189 通过](test/aicc_test/reports/acceptance/aicc-t1-2026-10-19T09-00-57-355Z-6782ed83/summary.md)
1. [T1 全矩阵报告：188 通过、1 失败](test/aicc_test/reports/acceptance/aicc-t1-2026-10-18T13-57-46-682Z-dd624b6f/summary.md)
2. [T1 失败项定向复验：1/1 通过](test/aicc_test/reports/acceptance/aicc-t1-2026-10-18T14-02-13-765Z-76ddd560/summary.md)
3. [Qwen Image 3 异步改造定向复验：3/3 通过](test/aicc_test/reports/acceptance/aicc-t1-2026-10-18T14-56-10-062Z-a1fb8df4/summary.md)

## 本轮 T1.5 报告

1. [2026-10-19 SN/OpenAI 全量：865/865 通过（SN 472、OpenAI 393）](test/aicc_test/reports/acceptance/t15-20261019091556-1238917/summary.md)
2. [2026-10-19 SN 单轮：466 passed、5 failed](test/aicc_test/reports/acceptance/t15-20261019090524-1224633/summary.md)
1. [SN 初次全量：299/451 通过](test/aicc_test/reports/acceptance/t15-20261018140456-3819810/summary.md)
2. [SN 第二次全量：344/460 通过](test/aicc_test/reports/acceptance/t15-20261018140858-3825383/summary.md)
3. [SN 最终全量：460/460 通过](test/aicc_test/reports/acceptance/t15-20261018141243-3830344/summary.md)
4. [OpenAI 受影响全量：393/393 通过](test/aicc_test/reports/acceptance/t15-20261018141529-3833843/summary.md)
5. [Qwen 异步协议全量：275/275 通过](test/aicc_test/reports/acceptance/t15-20261018145823-3894113/summary.md)

## 历史 T1 报告

1. [T1 全量报告 1](test/aicc_test/reports/acceptance/aicc-t1-2026-10-18T05-09-32-758Z-47402616/summary.md)
2. [T1 最终全量报告](test/aicc_test/reports/acceptance/aicc-t1-2026-10-18T06-27-49-158Z-dce1e035/summary.md)
3. [T1 修正后全量复验报告](test/aicc_test/reports/acceptance/aicc-t1-2026-10-18T08-21-04-357Z-ea94049f/summary.md)
4. [T1 资源分流修正后全量复验报告](test/aicc_test/reports/acceptance/aicc-t1-2026-10-18T13-02-00-973Z-fcb1ada8/summary.md)

## 历史 T1.5 报告

1. [T1.5 全量报告 1](test/aicc_test/reports/acceptance/t15-20261018051413-3065093/summary.md)
2. [T1.5 定向报告 1](test/aicc_test/reports/acceptance/t15-20261018054009-3100276/summary.md)
3. [T1.5 全量报告 2](test/aicc_test/reports/acceptance/t15-20261018054032-3100755/summary.md)
4. [T1.5 初始化报告](test/aicc_test/reports/acceptance/t15-20261018063240-3191502/summary.md)
5. [T1.5 全量报告 3](test/aicc_test/reports/acceptance/t15-20261018063257-3191936/summary.md)
6. [T1.5 定向报告 2](test/aicc_test/reports/acceptance/t15-20261018065517-3222081/summary.md)
7. [T1.5 最终全量报告](test/aicc_test/reports/acceptance/t15-20261018065609-3223222/summary.md)
8. [T1.5 Qwen/SN 修正后定向复验报告](test/aicc_test/reports/acceptance/t15-20261018082600-3352909/summary.md)
9. [T1.5 Qwen 资源分流修正后定向复验报告](test/aicc_test/reports/acceptance/t15-20261018130610-3732887/summary.md)

## 2026-10-19 SN T2 详细报告

1. [定向十六单元：人工复核 16/16 通过](test/aicc_test/reports/acceptance/aicc-2026-10-19T09-42-55-268Z-f490ac88/summary.md) · [财务](test/aicc_test/reports/acceptance/aicc-2026-10-19T09-42-55-268Z-f490ac88/finance.md)
2. [定向七单元：人工复核后 6 passed、1 failed](test/aicc_test/reports/acceptance/aicc-2026-10-19T09-30-39-700Z-e72f3108/summary.md) · [财务](test/aicc_test/reports/acceptance/aicc-2026-10-19T09-30-39-700Z-e72f3108/finance.md)
1. [全量 80 单元：原始 10 passed、48 failed、22 review；含人工复核及审计补充](test/aicc_test/reports/acceptance/aicc-2026-10-19T07-28-29-275Z-eb259b9a/summary.md) · [财务](test/aicc_test/reports/acceptance/aicc-2026-10-19T07-28-29-275Z-eb259b9a/finance.md)
2. [42 单元：原始 8 passed、14 failed、20 review；人工复核后 24 passed、15 failed、3 review](test/aicc_test/reports/acceptance/aicc-2026-10-19T07-48-16-711Z-bac3f0f4/summary.md) · [财务](test/aicc_test/reports/acceptance/aicc-2026-10-19T07-48-16-711Z-bac3f0f4/finance.md)
3. [有效 computer-use 六单元：2 个后端失败、4 个熔断拦截](test/aicc_test/reports/acceptance/aicc-2026-10-19T08-14-26-156Z-e3db2fbb/summary.md) · [财务](test/aicc_test/reports/acceptance/aicc-2026-10-19T08-14-26-156Z-e3db2fbb/finance.md)
4. [computer-use 四单元：2 个后端失败、2 个熔断拦截](test/aicc_test/reports/acceptance/aicc-2026-10-19T08-15-30-112Z-b647e6ac/summary.md) · [财务](test/aicc_test/reports/acceptance/aicc-2026-10-19T08-15-30-112Z-b647e6ac/finance.md)
5. [computer-use 最后两单元：2 个后端失败，全部单元均已有有效调用结果](test/aicc_test/reports/acceptance/aicc-2026-10-19T08-17-49-401Z-601783fc/summary.md) · [财务](test/aicc_test/reports/acceptance/aicc-2026-10-19T08-17-49-401Z-601783fc/finance.md)

## Qwen 定向 T2 报告

1. [Qwen Image 3 异步定向实测：2 项交付成功、人工复核 2/2 通过](test/aicc_test/reports/acceptance/aicc-2026-10-18T15-05-43-442Z-1e4770de/summary.md)
2. [Qwen Image 3 图生图旧同步实现复测：0/1 通过](test/aicc_test/reports/acceptance/aicc-2026-10-18T14-27-33-200Z-2387a270/summary.md)

## 历史 T2 报告

1. [T2 组合计划报告](test/aicc_test/reports/acceptance/aicc-2026-10-18T06-10-16-048Z-1222211d/summary.md)
2. [SN 初始计划报告](test/aicc_test/reports/acceptance/aicc-2026-10-18T06-10-41-691Z-279cc3b4/summary.md)
3. [SN 全量计划报告](test/aicc_test/reports/acceptance/aicc-2026-10-18T06-12-17-208Z-321e464b/summary.md)
4. [SN 全量实测报告](test/aicc_test/reports/acceptance/aicc-2026-10-18T06-12-32-686Z-f0b7be37/summary.md)
5. [受影响 Provider 矩阵计划报告](test/aicc_test/reports/acceptance/aicc-2026-10-18T06-15-25-965Z-e0b366fe/summary.md)
6. [受影响 Provider 矩阵实测报告](test/aicc_test/reports/acceptance/aicc-2026-10-18T06-15-37-799Z-17c37b57/summary.md)
7. [GLM 与 Qwen Image 定向报告](test/aicc_test/reports/acceptance/aicc-2026-10-18T06-17-28-063Z-1ea46d5b/summary.md)
8. [Qwen Image 3 定向报告](test/aicc_test/reports/acceptance/aicc-2026-10-18T07-24-54-029Z-2621734c/summary.md)
9. [Qwen Image 3 图生图复测报告](test/aicc_test/reports/acceptance/aicc-2026-10-18T07-27-08-006Z-88c38b0e/summary.md)
