# 火山方舟与豆包语音模型库存（2026-11-16）

## 官方依据与渠道边界

标准渠道 `doubao` 继续通过有鉴权的 `GET https://ark.cn-beijing.volces.com/api/v3/models` 动态发现，不改为静态白名单。本次响应有 55 个条目：39 个 Published、16 个 Retiring。39 个 Published 包含 3 个 3D 模型和 1 个 Router，现有排除规则继续生效。Retiring 条目沿用发现层的不可用/废弃标记，不删除原厂参数、渠道映射或历史适配。

官方来源：

- [火山方舟模型列表](https://ark.volcengine.com/region:cn-beijing/docs/model-list.md)
- [模型发布公告](https://docs.volcengine.com/docs/ark/model-release-announcement?lang=zh)
- [图片生成 API](https://ark.volcengine.com/region:cn-beijing/docs/image-generation-api.md)
- [Seedance 2.5 系列教程](https://ark.volcengine.com/region:cn-beijing/docs/seedance-2-5.md)
- [豆包语音模型列表](https://docs.volcengine.com/docs/6561/2499930?lang=zh)
- [录音文件识别闲时版任务提交](https://docs.volcengine.com/docs/6561/2608618?lang=zh)与[结果查询](https://docs.volcengine.com/docs/6561/2608619?lang=zh)

六个相关标准渠道模型的原始公开模型描述保存在 `test/aicc_test/acceptance/fixtures/doubao-models.json`；其中 Seedream 5.0 Flash 是上一轮已接入的模型，本轮保留其回归覆盖。该文件不包含鉴权信息。

## 新增映射

| 渠道 | 物理模型 ID | 原厂模型 | AICC API |
| --- | --- | --- | --- |
| doubao | kimi-k3-1-pro-261024 | kimi/kimi-k3.1-pro | llm、vision.ocr、vision.caption |
| doubao | deepseek-v4-1-flash-261017 | deepseek/deepseek-v4.1-flash | llm |
| doubao | deepseek-v4-1-flash-economy-261017 | deepseek/deepseek-v4.1-flash-economy | llm |
| doubao | doubao-seedream-5-5-261028 | doubao/doubao-seedream-5.5 | image.txt2img、image.img2img |
| doubao | doubao-seedance-2-5-flash-261115 | doubao/doubao-seedance-2.5-flash | video.generate、video.edit、video.extend |
| doubao-speech | doubao-seed-asr-2.0-idle | doubao/doubao-seed-asr-2.0-idle | audio.asr |

标准渠道两个新 DeepSeek ID 的实时 `task_type` 只有 `text_generation`，因此渠道发现结果收窄为 llm；不据此删改原厂模型的视觉能力。Kimi 与 DeepSeek 复用已核对的原厂参数，只补标准渠道身份和 reasoning 预设映射。

Seedream 5.5 生成单图，按原厂文档省略 `sequential_image_generation` 及其选项，沿用现有单图 lowering 配置。未新增图层拆分、RGBA 或富文本 OCR 的 canonical 接口。默认图片尺寸使用既有 codec 的 2K，处于官方 1K/1.5K/2K 范围内。

Seedance 2.5 Flash 原厂能力写入 `doubao.model.json`：最多 10 张参考图片、5 个参考视频、5 个参考音频，视频/音频参考总时长分别最多 30 秒，输出 4–30 秒、480p/720p。不继承普通 2.5 的 30/10/10 数量或 1080p 输出。渠道复用已有视频 codec，并通过 request rules 为 edit/extend 设置官方任务类型。

## 语音渠道

当前可选库存：

- `doubao-seed-tts-2.0`：TTS 2.0，`seed-tts-2.0`。
- `doubao-seed-asr-2.0`：录音识别标准版，`volc.seedasr.auc`。
- `doubao-seed-asr-2.0-fast`：录音识别极速版，`volc.bigasr.auc_turbo`。
- `doubao-seed-asr-2.0-idle`：录音识别闲时版，`volc.bigasr.auc_idle`。

闲时版通过新增操作 `asr.recognize.idle.submit` 复用已有异步 ASR codec，在 Known Provider 配置其 base URL 为 `/api/v3/auc/bigmodel/idle`，请求路径为 `/submit`、`/query`。请求头资源 ID 在 `doubao-speech.provider.json` 配置。无需修改公共执行、任务、协议类型或依赖。

官方说明闲时任务通常在 24 小时内完成，高峰可能更久；短时验收无法保证观察到完成结果，应如实记录超时。原有 ICL 2.0 保留库存条目和排除规则：它需要预先开通、注册复刻音色，当前 AICC 未实现完整资源管理。官方另列的 Seed Audio 1.0、WebSocket 流式识别和实时语音对话依赖不同接口，当前不冒充为已有 HTTP TTS/ASR 可调用模型。

## 验证与交付边界

新增模型已加入独立官方能力基线和 T1.5 协议矩阵；闲时版有独立 submit/query 路径、资源头和异步状态契约，Seedream 5.5 禁止发送组图参数。新增 Rust 测试验证物理 ID 到原厂身份及 operation 的解析、Flash 视频限制、闲时版提交与查询使用相同任务 ID 和正确资源头。

本轮不修改已有原厂模型参数、不修改 Jarvis 提示词或 cmd_skills。检查了 `aigc_image`、`aigc_audio`、`aigc_video`、`xllm` 的联动范围：本轮复用既有 canonical 调用、任务结果和资源引用方式，无新增用户命令或调用字段；模型名称与渠道能力的更新记录在本文和验收基线，遵照用户要求不编辑 Jarvis 手册。

T1.5 必须使用包含新增闲时 operation 的 AICC 二进制。T2 仅在对应 T1.5 通过且本轮真实调用范围获授权后执行；未运行的层次不能作为已通过结果。

本轮已通过：

- `cargo test --manifest-path src/Cargo.toml -p aicc --lib doubao -- --test-threads=1`：28/28。
- `node --experimental-strip-types --test acceptance/acceptance.test.ts`：163/163。
- `node --experimental-strip-types acceptance/preflight.ts`：通过，完整 manifest 1361 项 T1.5。
- `cargo build --manifest-path src/Cargo.toml -p aicc`：通过。
- 选定新增模型的 T1.5 manifest dry-run：标准渠道 35 项（含 17 个独立预设 cell），语音闲时版 14 项；dry-run 无网络请求、无 DV 配置变更。

已安装新二进制到 `/opt/buckyos/bin/aicc/aicc`，SHA256 为 `2adeaca31838bee9dc64f33e201303a86536efff62deba690b7efca9af9f8add`，旧文件备份为 `aicc.before-standard-speech-20261116`。已由用户完成重启。网关启动时短暂出现 3180 绑定冲突，随后自行恢复，无需修改启动配置。该 T1.5 阶段真实推理调用数为 0；后续 T2 记录见下。未执行全 workspace 测试或完整系统构建。

### 网关 T1.5 与测试范围修复

标准渠道首轮 39 项中 35 项通过、4 项失败。失败项均为本次没有选中的旧 Seedream 5.0 Flash：runner 的 `--model` 仅过滤官方库存和预设，未过滤 contract 的 `conformance_test_model_ids`，因而 manifest 引用了临时库存之外的模型。四项均未发出 Provider 请求，不属于模型接口失败。

修复 `scopeProtocolModels`，对每个 contract 的 conformance 模型池应用相同过滤，并增加断言：只选择 Seedream 5.5 时，恰好产生该模型的四个 success/artifact 单元，原始目录保持不变。该修复仅涉及验收 runner，无需重建部署；163 项自测、Deno 类型检查及静态 preflight 再次通过。

标准渠道最终重跑 `t15-20261116060218-635203`：35/35 通过，包含本轮新增模型的 API、17 个独立 reasoning 预设和制品结果校验；临时 Provider 清理通过，真实调用为 0。

语音闲时版首轮 `t15-20261116060019-632966`：14/14 通过，包括提交、查询、队列超时、错误和响应解析；临时 Provider 清理通过，真实调用为 0。测试 runner 修复后的最终复跑 `t15-20261116060324-636479` 同样为 14/14 通过，临时 Provider 清理通过，真实调用为 0。两组最终合计 49/49 通过。

本轮 T2 离线方案：上述六个新模型共 11 个 `ProviderInstance × model × API-Type` 单元，另需 1 次图片和 1 次视频前置生成，最多 13 次真实调用、0 次重试、并发 1、间隔 1 秒、每单元 300 秒、估算预算上限 3 USD；不启用 Judge。计划临时注入并恢复标准渠道与语音渠道的凭证/设置，通过 Zone Gateway 执行，不向外发送消息。该完整方案未执行；用户随后要求跳过已有 Agent Plan 成功记录的模型，每个剩余模型只选择一个渠道，实际执行范围如下。

### 按模型去重后的真实 T2（2026-11-16）

按用户新要求，不重复 Agent Plan 已成功的原厂模型：

- `deepseek-v4.1-flash`：沿用 Agent Plan 已有 llm 成功记录，例如 `aicc-2026-09-29T14-06-14-898Z-c836c51b`，不因标准渠道新增版本映射重复测试。
- `deepseek-v4.1-flash-economy`、`kimi-k3.1-pro`、`doubao-seedream-5.0-flash`：沿用上一轮 Agent Plan 成功及人工复核记录，见 `doubao_agent_plan_inventory.md`。

本次选定标准渠道 `doubao-seedream-5-5-261028`（两项图片 API）、`doubao-seedance-2-5-flash-261115`（三项视频 API），以及 speech 渠道 `doubao-seed-asr-2.0-idle`（ASR），共六个单元，含前置生成最多八次调用；每次最多一次尝试、并发一、间隔一秒、估算总预算三美元，无 Judge 或外部消息。

| 模型 | 渠道 | T2 结果 |
| --- | --- | --- |
| Seedream 5.5 | 标准 | 账号未开通，官方 `ModelNotOpen`；未生成图片，编辑所需前置图片也被同一限制拒绝 |
| Seedance 2.5 Flash | 标准 | 账号未开通，官方 `ModelNotOpen`；未生成视频，编辑/延长前置视频被同一限制拒绝 |
| ASR 2.0 闲时版 | speech | 成功；人工复核通过，转写为“今天的测试编号是4827。” |

首次运行 `aicc-2026-11-16T07-09-38-969Z-65b10ddb` 计入五次调用。标准渠道的五个 API 单元均由基础生成或共享前置生成的 `ModelNotOpen` 阻断，不属于 AICC 协议缺陷。ASR 首次请求返回 404，原因是本地测试配置生成时把 runner 要求的 `operation=url` 字符串数组误写成嵌套表，未正确覆盖 operation base URL；不是产品 metadata 或 codec 错误。

只修正被 Git 忽略的本地 TOML，并在补测脚本中校验解析后的 idle endpoint，再执行一次 speech-only T2：`aicc-2026-11-16T07-13-13-263Z-c3121b1b`。它在约 8.6 秒内成功，任务 ID `t-c84c1ac3afa94186b71c0e6e090868ae`，报告音频时长 3.533 秒，转写匹配夹具说明中的“今天的测试编号是四八二七”。原机器报告保留 `review`，人工复核结论追加到本地 `summary.md`。

两轮合计六次真实调用，未超过八次上限；实际费用均未返回，不能将报告的已知费用聚合 0 解释为免费，累计未知费用估算敞口 0.60 USD。两轮都恢复临时 Provider 凭证/设置并清理各自的 26 个上传夹具对象；没有生成媒体制品。未修改产品实现、Mock、Jarvis 提示词或手册。

当前未通过项为标准账号缺少两个媒体模型的开通权限。Agent Plan 当前官方库存不包含这两个模型，speech 不提供图片/视频接口，因而没有同模型的可用替代渠道。需要在方舟控制台开通对应 Model ID，或使用已有权限的标准账号凭证后定向继续；不开通时保留为账号受限，不能宣称两个模型 T2 通过。

本地原始报告目录为 `test/aicc_test/reports/acceptance/doubao-new-uncovered-20261116/` 与 `test/aicc_test/reports/acceptance/doubao-new-uncovered-speech-20261116/`；本次只提交本文的脱敏概要，不提交逐项报告、原始任务响应或本地配置。
