# 图片契约与结果处理

文生图使用 `images.generate` / `helper.text_to_image`；图生图使用 `image.img2img`。Rust 定义在 `src/kernel/buckyos-api/src/aicc_client.rs`，SDK 定义在 `buckyos-websdk/src/aicc_client.ts`。

| 参数 | 文生图 | 图生图 |
| --- | --- | --- |
| 提示词 | `prompt` | `prompt` |
| 参考图 | 无 | `images` |
| 比例 | `aspect_ratio` | `aspect_ratio` |
| 尺寸 | `size` | `output.size` |
| 负面提示词、质量、种子 | `negative_prompt`、`quality`、`seed` | 同左 |
| 输出格式 | `output.media_type` | 同左 |
| 变换强度 | 无 | `strength` |

图生图拒绝顶层 `size`。比例是两边为 1–100 的 `W:H`，像素尺寸是正整数 `WxH`；显式像素尺寸与比例必须一致。`1K`、`2K`、`4K` 和 `auto` 只是可解析的值，具体渠道仍会校验其支持范围。

## 模型选择与校验

参数不参与路由筛选。先按原有路由配置选定模型，再由选定 operation 校验参数；不支持时返回结构化 `unsupported_operation`，指出渠道、模型和字段，在发起上游推理前失败。不静默忽略，不改写提示词，不自动换候选模型。`models.list` 的 `capabilities` 中，`image.img2img.output.size: true` 等键描述适配器的字段映射，不保证任意参数值适用于所有模型。

| 渠道 | 映射与限制 |
| --- | --- |
| OpenAI / SN | 比例转换为 `size`。GPT Image 2 系列使用 16 倍数的像素尺寸，比例限制在 1:3–3:1；GPT Image 1 / 未指定图片模型的 Responses 工具使用 1024x1024、1536x1024、1024x1536。兼容渠道的其他图片模型保留显式像素尺寸，不套用 GPT Image 1 的尺寸枚举。拒绝负面提示词、种子和强度。 |
| Gemini Interactions | 比例和尺寸映射到 `response_format`，种子到 `generation_config.seed`。尺寸仅支持 1K/2K/4K；拒绝负面提示词、质量和强度。 |
| OpenRouter | 比例、尺寸、质量、种子和输出格式按 Image operation 映射；拒绝负面提示词和强度。 |
| Seedream | 比例按已有尺寸表转换，显式尺寸优先；支持种子。拒绝质量、负面提示词和强度。 |
| Qwen | 文生图 `size`、图生图 `output.size` 转为 `W*H`，负面提示词和种子进入 `parameters`；Wanx 文生图负面提示词在 `input`。Qwen 2.1 不支持负面提示词。拒绝比例、质量和强度。 |
| MiniMax | 支持比例；文生图支持数量。拒绝未映射的尺寸、输出格式、负面提示词、质量、种子和强度。 |
| fal Queue | 保留负面提示词、种子、数量及编辑强度映射；拒绝未映射的比例、尺寸、质量和输出选项。 |

协议依据：[OpenAI Images](https://developers.openai.com/api/reference/resources/images/methods/generate)、[OpenAI Responses](https://developers.openai.com/api/docs/guides/image-generation)、[Qwen 编辑](https://www.alibabacloud.com/help/en/model-studio/qwen-image-edit-api)、[Qwen 生成与编辑](https://www.alibabacloud.com/help/en/model-studio/qwen-image-generation-and-editing-api-reference)、[Gemini Interactions](https://ai.google.dev/api/interactions-api)。

## AIWorkspace

`aiws.ai.image({ prompt, images?, aspect?, size?, quality?, negative?, seed?, format? })` 保持原有调用方式。有参考图时把尺寸放入 `output.size`，保留输出格式。

显式请求比例时，host 检查导入资源的实际尺寸，允许 5% 误差。尺寸缺失或比例错误返回 `AI_INVALID_ANSWER`，不写入成功缓存。旧缓存命中后若无法导入或比例不符，按本次预算重新生成并替换缓存；有效缓存不消耗生成预算。资源导入识别 WebP 的 VP8、VP8L、VP8X 尺寸，避免有效 WebP 因缺失尺寸被反复拒绝。

`unsupported_operation` 在 SDK、CLI 和 Desktop 中保留为 `AI_UNSUPPORTED_OPERATION`；用户应修改参数或显式更换模型。WASM 与服务端将其视为需要干预的终态，避免自动重复执行。此变更不负责自动裁剪、拉伸或跨步骤画幅规划。
