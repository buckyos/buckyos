# SN T2 语义复核与最终限制

复核日期：2026-10-21。仅回读已完成任务与本地夹具；未调用额外模型或 judge。自动报告中的 `review` 状态保持原样，本文件提供独立语义结论。

夹具：[描述图片](../jarvis_media_dv/assets/image_primary.png)、[OCR 图片](../jarvis_media_dv/assets/image_ocr.png)、[语音](../jarvis_media_dv/assets/audio_speech.wav)。OCR 最小标准要求包含 `4827`，不要求完整前缀逐字一致；语音原文为“今天的测试编号是四八二七”。

## 首轮语义结果

对应[真实调用报告](reports/acceptance/aicc-2026-10-21T11-15-39-265Z-105ab5fa/summary.md)。9 项复核：通过 8、失败 1。

| 模型 / API | 结论 | 证据与限制 | TaskMgr ID |
|---|---|---|---|
| `kimi-k3.vision.caption` | 通过 | 正确描述粉色花、黄色花心、绿色茎叶及棕色椭圆背景。 | `t-397964cd1bf3467196df5c32ea6e0d32` |
| `kimi-k3.vision.ocr` | 通过 | 输出 `BUCKY05-0V-4827`，包含验收标记 `4827`；前缀存在字符识别误差。 | `t-e83c903ea29b4cf88fde0f32e769e3ec` |
| `qwen3.6-plus.vision.caption` | 通过 | 正确识别花朵的颜色、茎叶和背景几何构图。 | `t-d5c2e2140f654d7494fc07b0037ca383` |
| `qwen3.7-plus.vision.caption` | 通过 | 正确识别粉色花、黄色花心、绿叶和分层背景。 | `t-02c03a19bbda4d9b985bbcaaeee6407e` |
| `qwen3.7-plus.vision.ocr` | 失败 | 输出 `BUCHfiBg-DlU-qBB7`，未包含必需标记 `4827`；属于模型输出质量失败。 | `t-93d5080ec1f544b3b4ecc495f6376672` |
| `qwen3.8-max-0902.vision.caption` | 通过 | 正确描述主体、颜色、背景与平面插画风格。 | `t-f00bc130256942cfa82b1c3fdbc93b75` |
| `ui-tars-1.5-7b.vision.caption` | 通过（有质量限制） | 主体花朵、黄色花心、绿叶及棕色椭圆识别正确；白色背景和渐变描述不准确。按当前最小主体描述标准通过，不代表逐句事实准确。 | `t-8041dbc368c248f5945ccd3e5c7e9082` |
| `ui-tars-1.5-7b.vision.ocr` | 通过 | 输出 `BUCKY705-DV-4827`，包含验收标记 `4827`；前缀存在字符识别误差。 | `t-16e29472aa6c4ffd95c34354aa08a033` |
| `voxtral-small-24b-2507.audio.asr` | 通过 | 输出“今天的测试编号是 4827。”，与音频内容一致，允许中文数字与阿拉伯数字等价。 | `t-d67d91c992d445098d177a017ea2da9c` |

## Wan 上游限制

`wan-3.0` 的 `video.txt2video` 与 `video.img2video` 均已实际提交并失败，上游返回 HTTP 400：`Content-Type must be application/json`。不计为通过，也未跳过或替换成其他 provider。

SN [release 配置](https://github.com/buckyos/sn-business/blob/main/CD/cluster_config/release/apps.ts)将 Wan 绑定为 `openai/alibaba/wan-3.0`，API base 指向 OpenRouter。[LiteLLM OpenAI 视频适配器](https://github.com/BerriAI/litellm/blob/main/litellm/llms/openai/videos/transformation.py)固定使用 multipart；[OpenRouter 官方视频接口](https://openrouter.ai/docs/api/api-reference/video-generation/create-videos)要求 JSON，并使用 `duration`、`frame_images` 等字段。SN 对外的 OpenAI/LiteLLM 请求契约与其当前上游绑定之间存在适配缺口，需要 SN 部署侧修复。修改 AICC 输入编码无法改变 LiteLLM 的上游 multipart 编码。

AICC 的 SN 视频请求编码、状态/内容查询、错误终态及不可取消能力已通过 T1.5；Mock 的成功路径证明客户端契约，不能替代实际 SN→LiteLLM→OpenRouter 链路验证。Wan 的真实成功、视频下载及画面质量仍未验证。

## Qwen Image 最终结果

`qwen-image-3-pro` 的 `image.txt2img` 和 `image.img2img` 均失败。[定向报告](reports/acceptance/aicc-2026-10-21T11-24-49-991Z-2f3e63cf/summary.md)中的临时超时字段未在正确的网关节点生效；该轮任务最终记录非成功 HTTP 响应，AICC 记录 `OpenAI http_error: OpenAI request failed`。现有任务错误未保留原始响应体及具体 HTTP 状态，不据此推断具体 SN 内部故障节点。

`image.img2img` 按 AICC 二次创作验收要求执行 `image.txt2img` 前置生成，前置失败，尚未向真实图片编辑接口提交目标请求；不能将此单元视为编辑链路验证通过。自动报告将首轮裸 RPC 500 归入 AICC 缺陷候选，仅靠该通用分类不能确认产品归因。

末轮生成任务：`t-31ba7d5dfd154ddba78251cbdcbc323f`；二次创作前置任务：`t-49d41d9da6c14930a04ccd8c706b5952`。两者均未生成可供复核的输出制品。

## SN 鉴权与模型可见性（2026-10-23）

SN 设备身份登录返回的账户 KYC 状态为 `ACTIVE`，生效地区为 `US_FULL`，运行权限包含 OpenAI。设备登录后的 SN 原始 `/api/v1/ai/models` 目录包含 GPT 6、GPT 5.x、GPT Image、语音和 embedding；受限测试 key 的同一目录仅含 11 个非 OpenAI 模型。

`sn-ai-provider-main` 使用账户 `zz-dev-7` 的 `device_jwt` 动态登录，目录刷新成功；AICC 暴露 22 个基础物理模型、95 个含变体的条目，其中 OpenAI 基础模型为 19 个。Realtime 继续排除，Sora 继续排除。OpenAI T2 应固定到该实例，不向它注入受限测试 key。模型目录可见性不替代真实推理测试。

配置备份与原始诊断响应仅保存在本机，不提交凭据或账户私密材料。

## OpenAI 实例轮语义复核（2026-10-23）

对应[75 单元原始报告](reports/acceptance/aicc-2026-10-23T06-23-08-308Z-b2a88825/summary.md)。自动状态保留不改；以下复核使用本轮 TaskMgr 输出和清理前保存的制品，不调用额外 judge。

| 单元 | 复核 | 证据与限制 |
|---|---|---|
| `gpt-4o-mini-transcribe.audio.asr` | 通过 | 中文语音转写包含等价测试编号 4827。 |
| `gpt-4o-mini-tts.audio.tts` | 待复核 | 音频制品已下载；结构检查通过，尚未完成听辨。 |
| `gpt-4o-transcribe-diarize.audio.asr` | 通过 | 中文语音转写包含等价测试编号 4827。 |
| `gpt-5.3-codex.vision.caption` | 通过 | 识别花朵、主体颜色及茎叶；按最小主体描述标准通过，不代表逐句描述均准确。 |
| `gpt-5.3-codex.vision.ocr` | 通过 | 包含必需标记 4827；部分模型的完整前缀仍有误差。 |
| `gpt-5.4-mini.image.img2img` | 通过 | 逐项对照同模型前置原图：保留 4827 与中央构图，增加暖色光照；部分图像有材质/阴影变化。 |
| `gpt-5.4-mini.image.txt2img` | 通过 | 逐图查看：蓝色方形构图、清晰的白色 4827，满足最小生成要求。 |
| `gpt-5.4-mini.vision.caption` | 通过 | 识别花朵、主体颜色及茎叶；按最小主体描述标准通过，不代表逐句描述均准确。 |
| `gpt-5.4-mini.vision.ocr` | 失败 | 输出 BUCKY105-0V-4627，必需标记 4827 错为 4627。 |
| `gpt-5.4-nano.image.txt2img` | 通过 | 逐图查看：蓝色方形构图、清晰的白色 4827，满足最小生成要求。 |
| `gpt-5.4-nano.vision.caption` | 通过 | 识别花朵、主体颜色及茎叶；按最小主体描述标准通过，不代表逐句描述均准确。 |
| `gpt-5.4-nano.vision.ocr` | 失败 | 输出未包含 4827，字符识别不满足最小标准。 |
| `gpt-5.4-pro.image.txt2img` | 通过 | 逐图查看：蓝色方形构图、清晰的白色 4827，满足最小生成要求。 |
| `gpt-5.4-pro.vision.caption` | 通过 | 识别花朵、主体颜色及茎叶；按最小主体描述标准通过，不代表逐句描述均准确。 |
| `gpt-5.4-pro.vision.ocr` | 通过 | 包含必需标记 4827；部分模型的完整前缀仍有误差。 |
| `gpt-5.4.image.img2img` | 通过 | 逐项对照同模型前置原图：保留 4827 与中央构图，增加暖色光照；部分图像有材质/阴影变化。 |
| `gpt-5.4.image.txt2img` | 通过 | 逐图查看：蓝色方形构图、清晰的白色 4827，满足最小生成要求。 |
| `gpt-5.4.vision.caption` | 通过 | 识别花朵、主体颜色及茎叶；按最小主体描述标准通过，不代表逐句描述均准确。 |
| `gpt-5.4.vision.ocr` | 通过 | 包含必需标记 4827；部分模型的完整前缀仍有误差。 |
| `gpt-5.5-pro.image.txt2img` | 通过 | 逐图查看：蓝色方形构图、清晰的白色 4827，满足最小生成要求。 |
| `gpt-5.5-pro.vision.caption` | 通过 | 识别花朵、主体颜色及茎叶；按最小主体描述标准通过，不代表逐句描述均准确。 |
| `gpt-5.5-pro.vision.ocr` | 通过 | 包含必需标记 4827；部分模型的完整前缀仍有误差。 |
| `gpt-5.5.image.img2img` | 通过 | 逐项对照同模型前置原图：保留 4827 与中央构图，增加暖色光照；部分图像有材质/阴影变化。 |
| `gpt-5.5.image.txt2img` | 通过 | 逐图查看：蓝色方形构图、清晰的白色 4827，满足最小生成要求。 |
| `gpt-5.5.vision.caption` | 通过 | 识别花朵、主体颜色及茎叶；按最小主体描述标准通过，不代表逐句描述均准确。 |
| `gpt-5.5.vision.ocr` | 通过 | 包含必需标记 4827；部分模型的完整前缀仍有误差。 |
| `gpt-5.6-luna.image.img2img` | 通过 | 逐项对照同模型前置原图：保留 4827 与中央构图，增加暖色光照；部分图像有材质/阴影变化。 |
| `gpt-5.6-luna.image.txt2img` | 通过 | 逐图查看：蓝色方形构图、清晰的白色 4827，满足最小生成要求。 |
| `gpt-5.6-luna.vision.caption` | 通过 | 识别花朵、主体颜色及茎叶；按最小主体描述标准通过，不代表逐句描述均准确。 |
| `gpt-5.6-luna.vision.ocr` | 通过 | 包含必需标记 4827；部分模型的完整前缀仍有误差。 |
| `gpt-5.6-sol.image.img2img` | 通过 | 逐项对照同模型前置原图：保留 4827 与中央构图，增加暖色光照；部分图像有材质/阴影变化。 |
| `gpt-5.6-sol.image.txt2img` | 通过 | 逐图查看：蓝色方形构图、清晰的白色 4827，满足最小生成要求。 |
| `gpt-5.6-sol.vision.caption` | 通过 | 识别花朵、主体颜色及茎叶；按最小主体描述标准通过，不代表逐句描述均准确。 |
| `gpt-5.6-sol.vision.ocr` | 通过 | 包含必需标记 4827；部分模型的完整前缀仍有误差。 |
| `gpt-5.6-terra.image.img2img` | 通过 | 逐项对照同模型前置原图：保留 4827 与中央构图，增加暖色光照；部分图像有材质/阴影变化。 |
| `gpt-5.6-terra.image.txt2img` | 通过 | 逐图查看：蓝色方形构图、清晰的白色 4827，满足最小生成要求。 |
| `gpt-5.6-terra.vision.caption` | 通过 | 识别花朵、主体颜色及茎叶；按最小主体描述标准通过，不代表逐句描述均准确。 |
| `gpt-5.6-terra.vision.ocr` | 通过 | 包含必需标记 4827；部分模型的完整前缀仍有误差。 |
| `gpt-6-astra.image.img2img` | 通过 | 逐项对照同模型前置原图：保留 4827 与中央构图，增加暖色光照；部分图像有材质/阴影变化。 |
| `gpt-6-astra.image.txt2img` | 通过 | 逐图查看：蓝色方形构图、清晰的白色 4827，满足最小生成要求。 |
| `gpt-6-astra.vision.caption` | 通过 | 识别花朵、主体颜色及茎叶；按最小主体描述标准通过，不代表逐句描述均准确。 |
| `gpt-6-astra.vision.ocr` | 通过 | 包含必需标记 4827；部分模型的完整前缀仍有误差。 |
| `gpt-6-luna.image.txt2img` | 通过 | 逐图查看：蓝色方形构图、清晰的白色 4827，满足最小生成要求。 |
| `gpt-6-luna.vision.caption` | 通过 | 识别花朵、主体颜色及茎叶；按最小主体描述标准通过，不代表逐句描述均准确。 |
| `gpt-6-luna.vision.ocr` | 通过 | 包含必需标记 4827；部分模型的完整前缀仍有误差。 |
| `gpt-6-sol.image.txt2img` | 通过 | 逐图查看：蓝色方形构图、清晰的白色 4827，满足最小生成要求。 |
| `gpt-6-sol.vision.caption` | 通过 | 识别花朵、主体颜色及茎叶；按最小主体描述标准通过，不代表逐句描述均准确。 |
| `gpt-6-sol.vision.ocr` | 通过 | 包含必需标记 4827；部分模型的完整前缀仍有误差。 |
| `gpt-image-2.image.inpaint` | 失败 | 已生成绿色叶片，但部分超出中央蒙版，输出 1254×1254，原图 1024×1024；按相对坐标抽样，蒙版外 14080 点中 46 点最大通道差超过 10，未满足蒙版外保持不变的要求。 |
| `gpt-image-2.image.txt2img` | 通过 | 逐图查看：蓝色方形构图、清晰的白色 4827，满足最小生成要求。 |

## 其他 NativeTask provider 复核

[Qwen 轮](reports/acceptance/aicc-2026-10-23T07-43-48-586Z-19566e78/summary.md)：`qwen-image-3.0-pro.image.txt2img`、`wanx2.1-t2i-turbo.image.txt2img` 已逐图确认蓝底与白色 4827；`qwen-image-3.0-pro.image.img2img` 已对照本轮生成的源图，保留编号与构图并加入暖色光照，三项通过。`wan2.6-t2v.video.txt2video` 已完成任务及视频制品下载，画面语义未复核，保留 review。`qwen-image-edit.image.img2img` 上游返回 `InvalidParameter: Model not exist.`，失败。

[豆包语音轮](reports/acceptance/aicc-2026-10-23T07-47-52-194Z-8515748e/summary.md)：`doubao-seed-asr-2.0.audio.asr` 返回“今天的测试编号是4827。”，与原音频一致，语义复核通过。

## 已有 OpenAI 任务恢复验证

原 `gpt-5.4-nano.image.img2img` 任务 `t-e76b49ef0bf648f58aba17b5095a9dab` 已通过公网网关确认进入 `Terminal/Succeeded`；NDM 下载 1394623 字节图片，其 SHA-256 为 `e973b5e888a1f68729317c0f36afac8e249804dd39caa8f63004e7d99446b01b`，与本机保存的原上游响应图片一致。该检查仅调用 TaskMgr 查询和 NDM 读取，新增模型推理为 0。已查看恢复的目标图并与同模型源图对照：保留 4827 与主体构图，暖色光照明确，复核通过。原始 T2 超时报告保留，不改写。

[MiniMax 轮](reports/acceptance/aicc-2026-10-23T07-51-08-265Z-08d47181/summary.md)：`MiniMax-H3-Max` 与 `MiniMax-H3` 的文生视频、图生视频共 4 单元均已成功完成任务并下载视频。画面语义尚未复核，保留 review，不计最终通过。

[豆包方舟轮](reports/acceptance/aicc-2026-10-23T07-56-32-352Z-635bb417/summary.md)的 4 个 Seedance 物理模型 × 4 个 API，共 16 单元，均因所给账号未开通模型而受限；视频编辑/续写在本轮同模型生成前置受阻。[豆包 Agent Plan 轮](reports/acceptance/aicc-2026-10-23T07-56-52-096Z-e6c638ab/summary.md)的 16 单元亦受上游套餐/模型权限限制。未自动购买、开通或切换到其他 provider。逐单元错误见各轮报告，不计通过。

## SN 末次独立复测结论

- [Qwen OCR](reports/acceptance/aicc-2026-10-23T07-59-40-825Z-0d64a1a8/summary.md)：独立实例、网关 900 秒超时已确认生效，仍返回 `OpenAI http_error: OpenAI request failed`，未得到可复核文字，不计通过。该错误不足以确定 SN 内部故障节点。
- [Wan 四个 API](reports/acceptance/aicc-2026-10-23T08-01-49-226Z-ffba0489/summary.md)：文生视频与图生视频实际请求均返回 HTTP 400 `Content-Type must be application/json`；视频编辑/续写共享同模型生成前置，前置失败，未提交真实编辑/续写目标请求。
- [GPT Image 图生图](reports/acceptance/aicc-2026-10-23T08-01-59-912Z-5fcad118/summary.md)：固定激活实例动态登录，生成前置返回 `429 Insufficient credit balance`。原熔断阻塞已得到更明确的上游错误，但目标编辑仍未获成功验证。
- [既有 Nano 任务恢复详细报告](reports/acceptance/aicc-recovery-2026-10-23-sn-nano/summary.md)：公网读取验证成功，usage 仅 1 条，无新增推理。

末次所有测试实例均已移除，激活实例保留设备动态登录。非 SN 问题交由其他分支处理，本分支未保留 Fal 协议修改或非 SN 地域配置支持。

## 新 Key 定向复测与服务端源码核对（2026-10-23）

本轮只复测 10 个非质量失败单元。新 Key 的目录包含 OpenAI，不含 Qwen/Wan：OpenAI 使用新 Key 的独立实例；Qwen/Wan 继续使用此前 Key。激活实例保持动态登录。

| 目标 | 本轮结论 |
|---|---|
| `gpt-6-luna.image.img2img` | 调用、任务成功终态和制品下载通过，图像语义按用户要求未复核。 |
| `gpt-6-sol.image.img2img` | 调用、任务成功终态和制品下载通过，图像语义按用户要求未复核。 |
| `gpt-image-2.image.img2img` | 调用、任务成功终态和制品下载通过，图像语义按用户要求未复核。 |
| `qwen-image-3-pro.image.txt2img` / `image.img2img` | 仍返回通用 HTTP 错误；图生图被生成前置阻塞。 |
| `qwen3.7-plus.vision.ocr` | 仍在约 120 秒后返回通用 HTTP 错误，未获得文字结果。 |
| `wan-3.0.video.txt2video` / `video.img2video` | 仍返回 HTTP 400 `Content-Type must be application/json`。 |
| `wan-3.0.video.video2video` / `video.extend` | 同模型生成前置失败，未提交真实编辑/续写目标请求。 |

源码核对基于 `~/sn-business` 的 `fdfe184`，未修改该仓库、未发布 SN 服务。

1. **余额错误属于 SN 本地 AI Token 准入链路。** `src/modules/sn_aicc/src/http/service.rs:1364` 在余额不大于零时生成 `Insufficient credit balance.`；`http/error.rs:191` 将其编码为 429、`type=insufficient_balance`。设备账户只读余额查询确认当前余额已为负。原记录中的“上游额度不足”应具体理解为 SN 账户余额问题，不能据此宣称 OpenAI 供应商账户没余额或 KYC 无权限。新 Key 本轮三项均未再遇到此拒绝。
2. **Qwen 图片存在服务端下游路由冲突。** 线上只读 `model/info` 确认绑定 `openrouter/qwen/qwen-image-3-pro`；release 版本清单为 LiteLLM `1.103.0`。该版 [OpenRouter 图片适配器源码](https://raw.githubusercontent.com/BerriAI/litellm/v1.103.0/litellm/llms/openrouter/image_generation/transformation.py) 的 `get_complete_url` 固定走 `/chat/completions`，即使 AICC 调用的是 SN `/images/generations`，服务端仍会转换回 Chat。此前真实 Chat 请求已被该模型拒绝并要求 `/api/v1/images`。因此仅修改 AICC 的同步/异步选择不能修复此下游适配缺口。线上实际 wheel 版本及请求原始响应尚无服务端登录权限核对，不能断言这解释了全部 120 秒错误。
3. **Wan 的请求编码冲突明确。** 线上 `model/info` 确认绑定 `openai/alibaba/wan-3.0` 与 OpenRouter api_base。[LiteLLM 1.103.0 OpenAI 视频适配器](https://raw.githubusercontent.com/BerriAI/litellm/v1.103.0/litellm/llms/openai/videos/transformation.py) 的 `use_multipart_form_data` 返回 true；实际上游要求 JSON。SN 的 `src/apps/litellm/video_compat.py` 仅修正响应 `object/error`，未修正请求编码。后续服务端修复还应核对 OpenRouter 字段映射，不能只替换 Content-Type 而保留 multipart body。
4. **Qwen OCR 的 120 秒故障节点仍未确认。** 本地网关 900 秒已生效；SN 源码的 LiteLLM 默认超时为 600 秒。通用错误表明未解析到标准错误消息，不能仅据此确定是 SN 网关、LiteLLM 或更下游代理超时。需要对应时刻的服务端日志和原始 HTTP 状态/响应头，本机没有服务端登录权限。

本轮 T1.5 补跑 10 项 SN 制品结果回归，并新增 3 项 SN 视频上游 JSON 拒绝回归；末轮均通过。新增 fixture 验证上游拒绝能正确终结本地任务，不代表 SN 到 OpenRouter 的成功路径已修复。凭据隔离自测改为保留真实形态的 `device_jwt` 配置；131 项自测通过。所有轮次链接见汇总报告。

## Qwen 后台能力重新核实（2026-10-23）

此前“Qwen 的 Responses 必然被 LiteLLM 转成 Chat”判断撤回：LiteLLM 1.103.0 已注册 OpenRouter 原生 Responses 配置。线上 model/info 确认 Qwen3.7 路由为 openrouter/qwen/qwen3.7-plus，未见强制 Chat 覆盖；实际部署 wheel 版本仍未知。

一次授权的文本诊断使用 background=true、store=false，约 2.16 秒后得到完整 OK；同 Key 查询返回 ID，上游 OpenRouter 返回 404。短请求不能证明 background 被忽略，但该短请求未观察到排队状态，查询完成响应失败不能证明长任务异步不可用。官方 SDK 接受 background 参数，同时仅允许 store=false；概览明确无状态。不能据 schema 有参数就声称后台任务可用，也不能外推为 Qwen 原厂不支持异步。详见[本轮诊断](reports/acceptance/aicc-diagnostic-2026-10-23-sn-qwen-background/summary.md)。

## OpenRouter 分支实现复核（2026-10-23）

核对本地分支 `test/openrouter-retest-20261015`，HEAD `f158d8044`，未切换或合并该分支。

| 行为 | 实现与测试证据 | 能证明什么 |
|---|---|---|
| Responses 的 background 参数 | `driver_metadata/providers/openrouter.provider.json:24` 为 google/gemini-4-argon 设置 background=true；`derived_responses.rs` 委托基础 codec 编解码 | 支持透传参数，不等于已实现后台轮询 |
| Responses 排队任务 | dialect 注册的 native_task_codecs 初始为空；OpenRouter 后续仅为视频增加 NativeTask codec；Responses contract 的 success_fixture 为 completed，未声明 async_protocol/async_steps | 此分支没有可直接复用的 Responses 排队/轮询实现或对应 T1.5 用例 |
| 视频异步 | `openrouter_media.rs:637` 实现 NativeTaskCodec，支持 Submit/Status/Result，POST videos、GET videos/{id} 与 content；strict Mock 具有 pending、轮询和结果阶段 | 视频具备真正原生异步实现，不能据此外推 Responses 后台能力 |
| 图片 | `image_descriptor` 仅声明 Immediate，使用独立 images API | 未实现图片后台提交/轮询 |
| Argon 历史 T2 | 2026-10-16 LLM/Caption/OCR 调用成功、review；报告保留 background=true 配置，但未提供 queued/in_progress 与后续查询证据 | 不能当作后台任务闭环已验证 |

以上是实现与测试覆盖判断，不是对上游能力的否定。短请求完成后返回的 response ID 也不能当作已验证的排队任务 ID。当前结论应为：需要有区分力的长任务诊断，而非直接判定后台不支持或直接新增 SN 队列。

## 原 Qwen OCR 用例开启 background（2026-10-23）

使用与原失败单元一致的 image_ocr.png（2189 bytes，SHA256 eaa912debbdd30ccbbebbdae7ac4950b321afaaf740dd98ad76af3d7cfdc19c9），base64 输入；临时 SN 独立实例将 vision.ocr 绑定已有 responses.background，发送 background=true/stream=false，没有关闭 thinking，也没有添加 store 参数。

| 轮次 | 输出上限 | 首次 AICC 响应耗时 | 实际结果 |
|---|---|---|---|
| 第一轮 | 1024 | 36612 ms | provider_error: max_output_tokens；未取得上游任务 ID，诊断上限提前终止，不作为超时结论 |
| 第二轮 | 4096 | 121194 ms | provider_error: OpenAI http_error: OpenAI request failed；未取得上游任务 ID |

第二轮本地网关响应等待临时设为 900 秒；没有进入原生状态查询阶段。该同负载实验说明本次开启 background 没有解决当前 SN 通道的约 120 秒错误，但没有取得原始 HTTP 状态/响应头和 SN 服务端日志，不能据此定位具体超时节点、判定 background 是否被下游忽略，或外推原厂不支持异步。第一轮输出上限失败也不足以证明额外 token 消耗必然来自 reasoning。

两轮共 2 次推理，无自动重发，费用未知，合并预算 US$0.10。settings/metadata、临时实例和本地网关均已恢复；未覆盖激活 SN 凭据。逐轮结果见汇总报告中的链接。

## 绕过 SN 直连 OpenRouter（2026-10-23）

按用户要求，以相同 OCR 图片和 background=true/stream=false/max_output_tokens=4096 直接 POST OpenRouter Responses。约 807ms 返回 HTTP 401 `User not found.`；同一凭据只读 GET /api/v1/key 亦返回同样 401。没有获得模型结果、usage 或后台任务 ID，没有重试。该结果仅说明本次凭据鉴权失败，不能比较 SN 和直连的长任务行为。详见[本轮报告](reports/acceptance/aicc-diagnostic-2026-10-23-openrouter-direct-qwen-ocr-background/summary.md)。
