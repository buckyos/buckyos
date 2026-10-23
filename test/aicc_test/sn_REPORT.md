# SN AICC 汇总测试报告

日期：2026-10-23（Asia/Shanghai）。环境：Ubuntu，运行目录 `/opt/buckyos`。源码基于 `53b4431e2d46ec731598ed0343d4a6596bee8d15` 的当前工作区。测试顺序为 T1 → T1.5 → T2；本分支修复范围为 SN provider。

## 最终汇总

| 验证范围 | 结果 |
|---|---|
| AICC 单元测试 | 当前源码 650/650 通过（Qwen OCR 恢复 Responses） |
| Acceptance runner 自测 | 131/131 通过 |
| 静态 preflight、相关 TypeScript 类型检查 | 通过 |
| AICC 模块构建 | 最新源码 `uv run buckyos-build.py -s aicc` 构建通过并已部署 |
| T1 全量 | 最新源码主用例 130/130、requirement branches 127/127、路由暴露 59/59 通过 |
| T1.5 全量及补充回归 | 最新源码 SN 全量 550/550；历史全量轮 2217/2217、SN 全量轮 548/548、受影响协议轮 494/494，补充制品 10/10、上游拒绝 3/3；均零真实调用，范围重叠不相加 |
| T2 SN 合并目标矩阵 | 30 个物理模型、99 个模型/API 单元：85 通过、10 失败、4 待语义复核 |
| 其中 OpenAI | 首轮激活实例、末轮新 Key 独立实例：19 个物理模型、75 单元，68 通过、3 质量失败、4 待语义复核 |
| 其中所给 SN Key | 独立测试实例：11 个物理模型、24 单元，17 通过、7 失败 |
| 既有异步任务恢复 | 原 GPT Nano 任务成功、制品哈希一致、usage 仅 1 条；新增推理 0 |
| 最终配置及环境 | 无测试 provider 残留；激活 SN 保持设备动态登录；网关配置按字节恢复并热重载，核心服务和端口可达 |

合并矩阵按凭据组、物理模型及 API 去重，采用每单元最后一次执行和独立语义复核结果；同一 Key 的临时实例名称变化不增加目标单元。原始自动报告保留，`review` 只有完成独立语义复核后才计通过。前置生成失败的编辑/续写单元计失败，不能视为目标接口已验证。

## 最终范围与限制

- SN 地址沿用激活时的 `https://sn.buckyos.ai/api/v1/ai`。OpenAI 首轮使用 `sn-ai-provider-main` 的 `device_jwt` 动态登录，末次 3 项定向复测使用新 Key 独立实例；原受限 Key 仅用于非 OpenAI 独立实例。设备账户 KYC 为 `ACTIVE`、运行地区为 `US_FULL`，已验证 OpenAI 目录及真实推理；目录权限不代表额度充足。
- Wan 已暴露 `video.txt2video`、`video.img2video`、`video.video2video`、`video.extend`，未再通过 `remove_api_types` 隐藏后两项。真实文生视频、图生视频均被 SN 上游以 HTTP 400 `Content-Type must be application/json` 拒绝；编辑/续写被同模型生成前置阻塞，尚未提交真实目标请求。
- Qwen Image 的文生图及图生图单元仍失败，错误为 `OpenAI http_error: OpenAI request failed`；图生图的生成前置失败，未验证真实编辑请求。已确认 Chat 路径被上游拒绝，保留专用图片接口；release 配置中的 LiteLLM 1.103.0 OpenRouter 图片适配器仍将专用图片请求转换到 Chat，存在服务端下游路由冲突；线上 model/info 已确认相同模型绑定，实际 wheel 版本尚未核验。
- Qwen 3.7 Plus OCR 常规配置已恢复 `responses.create`，常规路径尚未重跑 T2；最新临时 background 诊断仍约 121 秒返回 HTTP 错误；历史 Chat 独立实例也返回上游 HTTP 错误；该轮实际生效的网关响应超时为 900 秒，仍未通过。更早的 OCR 输出也未满足必需编号 `4827`。
- OpenAI 图生图 `gpt-6-luna`、`gpt-6-sol`、`gpt-image-2` 已用新 Key 完成调用、成功终态及制品下载；按用户要求未复核图像语义，三项保留 review。原 429 文案对应 SN 本地 AI Token 余额准入；只读查询确认原设备账户当前余额为负，不能归因于 KYC 或 OpenAI 供应商账户余额。
- `gpt-5.4-mini`、`gpt-5.4-nano` 的 OCR 未识别出 `4827`；GPT Image 局部重绘有蒙版外变化，均为质量失败。`gpt-4o-mini-tts` 尚未完成听辨，新 Key 的 3 项图生图尚未复核语义，共 4 项待复核。逐项结论见[独立语义复核](sn_T2_REVIEW.md)。
- `gpt-realtime-2.1` 按 [Realtime 模型说明](https://developers.openai.com/api/docs/models/gpt-realtime-2.1)继续排除；`sora-2`、`sora-2-pro` 按 [OpenAI 下线说明](https://developers.openai.com/api/docs/deprecations)继续排除。UI-TARS 原生 computer tool 继续排除。
- 未修改原厂 `.model.json`、其他 provider 的生产协议/配置、公共 DTO、SDK API 或依赖；SN 实测暴露的公共异步进度上报缺陷具有故障注入回归和真实既有任务恢复验证。媒体手册 `aigc_image`、`aigc_audio`、`aigc_video` 已核对，无需修改：Agent 使用逻辑模型路由，本次未改变其调用、任务恢复或结果交付方式。
- 已执行的非 SN 补充回归仅保留结果，不纳入本分支修复范围。Qwen 3 个图片单元和豆包语音已语义复核通过；部分视频、音频及图片制品待语义复核，豆包视频受账号/套餐限制，Fal 有失败及 usage 审计失败。不得据此宣称其他 provider 全部通过。
- 未修改 NDN 服务逻辑。临时网关超时仅用于真实测试，现已恢复。早期报告中的超时配置不能作为“900 秒已生效”的证据；末次独立 SN 复测使用了经网关运行配置确认的设置。

本次非质量失败定向复测共 10 单元：3 项调用及制品链路成功，7 项失败（含 3 项生成前置阻塞）。质量问题未重测。服务端源码诊断与未确认边界见[复核记录](sn_T2_REVIEW.md)。

Qwen 后台能力诊断：短文本请求的已完成响应查询 404 不足以判定异步支持；原 OCR 图片开启 background 后，1024-token 轮提前达到输出上限，4096-token 轮在本地网关 900 秒下仍约 121 秒返回 HTTP 错误。两轮均未取得上游任务 ID，本次未解决超时，未判定 Qwen 原厂异步能力。未改变原 99 单元矩阵；详见[诊断报告](reports/acceptance/aicc-diagnostic-2026-10-23-sn-qwen-background/summary.md)。

## 调用与费用

SN 各轮 runner 实际调用计数合计 142 次，另有后台能力诊断 3 次（短文本 1 次、原 OCR 图片 2 次），累计 145 次，含失败及生成前置；既有任务恢复不增加调用。SN 账本未知费用估算暴露合计 $1.48，实际结算费用未确认，不能把 known actual cost 为零解释为免费。短文本后台诊断另返回上游 reported_cost=0.0000064，不作为最终账单；两轮 OCR 诊断费用未知，合并预算 US$0.10 不作为实际费用。每单元最多一次尝试，各轮调用与预算上限均见财务报告。

非 SN 补充轮 runner 调用合计 39 次，单独计数，不加入 SN 矩阵；上游受限、RPC 失败及未完成审计的财务信息有局限，不能据此确定最终账单。

## 各轮详细报告

以下链接保留各轮实际执行的逐例结果；不同轮次的范围可能重叠或只覆盖部分用例。

| 层次/范围 | 本轮自动结果 | 逐例报告 | 财务 |
|---|---|---|---|
| T2 SN | passed=9；review=9；failed=4 | [aicc-2026-10-21T11-15-39-265Z-105ab5fa](reports/acceptance/aicc-2026-10-21T11-15-39-265Z-105ab5fa/summary.md) | [finance](reports/acceptance/aicc-2026-10-21T11-15-39-265Z-105ab5fa/finance.md) |
| T2 SN | failed=2 | [aicc-2026-10-21T11-24-49-991Z-2f3e63cf](reports/acceptance/aicc-2026-10-21T11-24-49-991Z-2f3e63cf/summary.md) | [finance](reports/acceptance/aicc-2026-10-21T11-24-49-991Z-2f3e63cf/finance.md) |
| T2 SN | failed=7 | [aicc-2026-10-23T06-20-06-095Z-7a916bff](reports/acceptance/aicc-2026-10-23T06-20-06-095Z-7a916bff/summary.md) | [finance](reports/acceptance/aicc-2026-10-23T06-20-06-095Z-7a916bff/finance.md) |
| T2 SN | review=50；passed=21；failed=4 | [aicc-2026-10-23T06-23-08-308Z-b2a88825](reports/acceptance/aicc-2026-10-23T06-23-08-308Z-b2a88825/summary.md) | [finance](reports/acceptance/aicc-2026-10-23T06-23-08-308Z-b2a88825/finance.md) |
| T2 补充 qwen | review=4；failed=1 | [aicc-2026-10-23T07-43-48-586Z-19566e78](reports/acceptance/aicc-2026-10-23T07-43-48-586Z-19566e78/summary.md) | [finance](reports/acceptance/aicc-2026-10-23T07-43-48-586Z-19566e78/finance.md) |
| T2 补充 doubao-speech | review=1 | [aicc-2026-10-23T07-47-52-194Z-8515748e](reports/acceptance/aicc-2026-10-23T07-47-52-194Z-8515748e/summary.md) | [finance](reports/acceptance/aicc-2026-10-23T07-47-52-194Z-8515748e/finance.md) |
| T2 SN | failed=7 | [aicc-2026-10-23T07-48-58-854Z-5dbe2934](reports/acceptance/aicc-2026-10-23T07-48-58-854Z-5dbe2934/summary.md) | [finance](reports/acceptance/aicc-2026-10-23T07-48-58-854Z-5dbe2934/finance.md) |
| T2 补充 minimax | review=4 | [aicc-2026-10-23T07-51-08-265Z-08d47181](reports/acceptance/aicc-2026-10-23T07-51-08-265Z-08d47181/summary.md) | [finance](reports/acceptance/aicc-2026-10-23T07-51-08-265Z-08d47181/finance.md) |
| T2 补充 doubao | provider_restricted=16 | [aicc-2026-10-23T07-56-32-352Z-635bb417](reports/acceptance/aicc-2026-10-23T07-56-32-352Z-635bb417/summary.md) | [finance](reports/acceptance/aicc-2026-10-23T07-56-32-352Z-635bb417/finance.md) |
| T2 补充 doubao-agent-plan | provider_restricted=16 | [aicc-2026-10-23T07-56-52-096Z-e6c638ab](reports/acceptance/aicc-2026-10-23T07-56-52-096Z-e6c638ab/summary.md) | [finance](reports/acceptance/aicc-2026-10-23T07-56-52-096Z-e6c638ab/finance.md) |
| T2 补充 fal | review=2；failed=3 | [aicc-2026-10-23T07-57-10-959Z-cfd52338](reports/acceptance/aicc-2026-10-23T07-57-10-959Z-cfd52338/summary.md) | [finance](reports/acceptance/aicc-2026-10-23T07-57-10-959Z-cfd52338/finance.md) |
| T2 SN | failed=1 | [aicc-2026-10-23T07-59-40-825Z-0d64a1a8](reports/acceptance/aicc-2026-10-23T07-59-40-825Z-0d64a1a8/summary.md) | [finance](reports/acceptance/aicc-2026-10-23T07-59-40-825Z-0d64a1a8/finance.md) |
| T2 SN | failed=4 | [aicc-2026-10-23T08-01-49-226Z-ffba0489](reports/acceptance/aicc-2026-10-23T08-01-49-226Z-ffba0489/summary.md) | [finance](reports/acceptance/aicc-2026-10-23T08-01-49-226Z-ffba0489/finance.md) |
| T2 SN | failed=1 | [aicc-2026-10-23T08-01-59-912Z-5fcad118](reports/acceptance/aicc-2026-10-23T08-01-59-912Z-5fcad118/summary.md) | [finance](reports/acceptance/aicc-2026-10-23T08-01-59-912Z-5fcad118/finance.md) |
| T1 | passed=186；failed=3 | [aicc-t1-2026-10-21T09-30-02-069Z-902b531d](reports/acceptance/aicc-t1-2026-10-21T09-30-02-069Z-902b531d/summary.md) | [finance](reports/acceptance/aicc-t1-2026-10-21T09-30-02-069Z-902b531d/finance.md) |
| T1 | passed=189 | [aicc-t1-2026-10-21T09-45-44-405Z-faddf15f](reports/acceptance/aicc-t1-2026-10-21T09-45-44-405Z-faddf15f/summary.md) | [finance](reports/acceptance/aicc-t1-2026-10-21T09-45-44-405Z-faddf15f/finance.md) |
| T1 | passed=189 | [aicc-t1-2026-10-21T10-31-40-241Z-d31b0961](reports/acceptance/aicc-t1-2026-10-21T10-31-40-241Z-d31b0961/summary.md) | [finance](reports/acceptance/aicc-t1-2026-10-21T10-31-40-241Z-d31b0961/finance.md) |
| T1 | passed=189 | [aicc-t1-2026-10-21T14-24-57-288Z-278ae4f4](reports/acceptance/aicc-t1-2026-10-21T14-24-57-288Z-278ae4f4/summary.md) | [finance](reports/acceptance/aicc-t1-2026-10-21T14-24-57-288Z-278ae4f4/finance.md) |
| T1 | passed=189 | [aicc-t1-2026-10-23T07-09-02-881Z-550df37a](reports/acceptance/aicc-t1-2026-10-23T07-09-02-881Z-550df37a/summary.md) | [finance](reports/acceptance/aicc-t1-2026-10-23T07-09-02-881Z-550df37a/finance.md) |
| T1.5 | passed=2205；failed=3 | [t15-20261021095106-141348](reports/acceptance/t15-20261021095106-141348/summary.md) | [finance](reports/acceptance/t15-20261021095106-141348/finance.md) |
| T1.5 | passed=2217 | [t15-20261021103754-219083](reports/acceptance/t15-20261021103754-219083/summary.md) | [finance](reports/acceptance/t15-20261021103754-219083/finance.md) |
| T1.5 | passed=546；failed=2 | [t15-20261021143022-542758](reports/acceptance/t15-20261021143022-542758/summary.md) | [finance](reports/acceptance/t15-20261021143022-542758/finance.md) |
| T1.5 | passed=546；failed=2 | [t15-20261023055326-3714404](reports/acceptance/t15-20261023055326-3714404/summary.md) | [finance](reports/acceptance/t15-20261023055326-3714404/finance.md) |
| T1.5 | failed=2 | [t15-20261023060446-3730172](reports/acceptance/t15-20261023060446-3730172/summary.md) | [finance](reports/acceptance/t15-20261023060446-3730172/finance.md) |
| T1.5 | failed=2 | [t15-20261023060702-3733260](reports/acceptance/t15-20261023060702-3733260/summary.md) | [finance](reports/acceptance/t15-20261023060702-3733260/finance.md) |
| T1.5 | failed=2 | [t15-20261023060757-3734502](reports/acceptance/t15-20261023060757-3734502/summary.md) | [finance](reports/acceptance/t15-20261023060757-3734502/finance.md) |
| T1.5 | failed=2 | [t15-20261023060833-3735339](reports/acceptance/t15-20261023060833-3735339/summary.md) | [finance](reports/acceptance/t15-20261023060833-3735339/finance.md) |
| T1.5 | passed=2 | [t15-20261023060935-3736780](reports/acceptance/t15-20261023060935-3736780/summary.md) | [finance](reports/acceptance/t15-20261023060935-3736780/finance.md) |
| T1.5 | passed=548 | [t15-20261023061029-3738027](reports/acceptance/t15-20261023061029-3738027/summary.md) | [finance](reports/acceptance/t15-20261023061029-3738027/finance.md) |
| T1.5 | passed=494 | [t15-20261023071827-3839409](reports/acceptance/t15-20261023071827-3839409/summary.md) | [finance](reports/acceptance/t15-20261023071827-3839409/finance.md) |
| T2 既有任务恢复 | passed=1；新增推理=0 | [Nano recovery](reports/acceptance/aicc-recovery-2026-10-23-sn-nano/summary.md) | [finance](reports/acceptance/aicc-recovery-2026-10-23-sn-nano/finance.md) |
| T2 SN | review=1 | [aicc-2026-10-23T08-35-22-957Z-f5aa06b7](reports/acceptance/aicc-2026-10-23T08-35-22-957Z-f5aa06b7/summary.md) | [finance](reports/acceptance/aicc-2026-10-23T08-35-22-957Z-f5aa06b7/finance.md) |
| T2 SN | review=1 | [aicc-2026-10-23T08-36-33-851Z-2281b6ee](reports/acceptance/aicc-2026-10-23T08-36-33-851Z-2281b6ee/summary.md) | [finance](reports/acceptance/aicc-2026-10-23T08-36-33-851Z-2281b6ee/finance.md) |
| T2 SN | review=1 | [aicc-2026-10-23T08-37-38-114Z-82f27bc8](reports/acceptance/aicc-2026-10-23T08-37-38-114Z-82f27bc8/summary.md) | [finance](reports/acceptance/aicc-2026-10-23T08-37-38-114Z-82f27bc8/finance.md) |
| T2 SN | failed=2 | [aicc-2026-10-23T08-38-07-831Z-fd140133](reports/acceptance/aicc-2026-10-23T08-38-07-831Z-fd140133/summary.md) | [finance](reports/acceptance/aicc-2026-10-23T08-38-07-831Z-fd140133/finance.md) |
| T2 SN | failed=1 | [aicc-2026-10-23T08-42-17-327Z-d43525dd](reports/acceptance/aicc-2026-10-23T08-42-17-327Z-d43525dd/summary.md) | [finance](reports/acceptance/aicc-2026-10-23T08-42-17-327Z-d43525dd/finance.md) |
| T2 SN | failed=4 | [aicc-2026-10-23T08-44-25-051Z-ebc020a4](reports/acceptance/aicc-2026-10-23T08-44-25-051Z-ebc020a4/summary.md) | [finance](reports/acceptance/aicc-2026-10-23T08-44-25-051Z-ebc020a4/finance.md) |
| T2 SN | failed=4 | [aicc-2026-10-23T08-46-59-378Z-65d86963](reports/acceptance/aicc-2026-10-23T08-46-59-378Z-65d86963/summary.md) | [finance](reports/acceptance/aicc-2026-10-23T08-46-59-378Z-65d86963/finance.md) |
| T1.5 SN | passed=10 | [t15-20261023083355-3941542](reports/acceptance/t15-20261023083355-3941542/summary.md) | [finance](reports/acceptance/t15-20261023083355-3941542/finance.md) |
| T1.5 SN | passed=3；failed=1 | [t15-20261023084435-3956049](reports/acceptance/t15-20261023084435-3956049/summary.md) | [finance](reports/acceptance/t15-20261023084435-3956049/finance.md) |
| T1.5 SN | passed=3 | [t15-20261023084638-3958825](reports/acceptance/t15-20261023084638-3958825/summary.md) | [finance](reports/acceptance/t15-20261023084638-3958825/finance.md) |
| T2 后台能力诊断 | 长任务异步未判定；短请求完成后查询 404；推理=1 | [Qwen background](reports/acceptance/aicc-diagnostic-2026-10-23-sn-qwen-background/summary.md) | [finance](reports/acceptance/aicc-diagnostic-2026-10-23-sn-qwen-background/finance.md) |
| T2 OCR 后台诊断 | max_output_tokens，36.6 秒；推理=1 | [aicc-diagnostic-2026-10-23-sn-qwen-ocr-background-1024](reports/acceptance/aicc-diagnostic-2026-10-23-sn-qwen-ocr-background-1024/summary.md) | [finance](reports/acceptance/aicc-diagnostic-2026-10-23-sn-qwen-ocr-background-1024/finance.md) |
| T2 OCR 后台诊断 | HTTP 错误，121.2 秒；推理=1 | [aicc-diagnostic-2026-10-23-sn-qwen-ocr-background-4096](reports/acceptance/aicc-diagnostic-2026-10-23-sn-qwen-ocr-background-4096/summary.md) | [finance](reports/acceptance/aicc-diagnostic-2026-10-23-sn-qwen-ocr-background-4096/finance.md) |
| OpenRouter 直连诊断 | HTTP 401 User not found；异步未验证；SN 调用=0 | [aicc-diagnostic-2026-10-23-openrouter-direct-qwen-ocr-background](reports/acceptance/aicc-diagnostic-2026-10-23-openrouter-direct-qwen-ocr-background/summary.md) | [finance](reports/acceptance/aicc-diagnostic-2026-10-23-openrouter-direct-qwen-ocr-background/finance.md) |
| T1 最新源码 | passed=189；主用例 130/130；分支 127/127；路由 59/59 | [aicc-t1-2026-10-23T10-37-38-115Z-2f4166c3](reports/acceptance/aicc-t1-2026-10-23T10-37-38-115Z-2f4166c3/summary.md) | [finance](reports/acceptance/aicc-t1-2026-10-23T10-37-38-115Z-2f4166c3/finance.md) |
| T1.5 SN 最新源码 | passed=550；failed=0；覆盖 550/550 | [t15-20261023104257-4125885](reports/acceptance/t15-20261023104257-4125885/summary.md) | [finance](reports/acceptance/t15-20261023104257-4125885/finance.md) |

各 T2 轮的 `effective_baseline.json` 与报告同目录保存。逐例 JSON、原始响应、凭据和下载制品仅保留本地，不纳入 Git。
