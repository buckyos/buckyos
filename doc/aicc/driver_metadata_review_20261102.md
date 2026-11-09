# SN 新增原厂模型参数复核（2026-11-02）

本次新增原厂定义为 5 个；SN 新增目录条目还包含别名和推理预设，不能按目录条目数算原厂模型数。复核基于以下原厂公开说明。原厂事实放在 Model Driver；SN/OpenRouter 输出额度继续保留在各自 Provider Rules，不回写原厂文件。

| 原厂模型 | 核实参数 | 配置结论 |
|---|---|---|
| GPT Image 2.5 Flare、Sunburst | 文本/图像输入，图片输出；无流式、工具调用、结构化输出；quality=auto/low/medium/high/xhigh/max；size=auto 或自定义，边长为 16 倍数且 ≤3840，长宽比 ≤3，像素数 655360–8294400，超过 3686400 为实验规格；n=1–10；背景 auto/opaque/transparent；PNG/JPEG/WebP | 补齐 `image_generation_parameters`，显式声明 tool_call/json_schema=false。保留 txt2img/img2img/inpaint；原厂 OpenAI 的 generate/edit 映射和 SN 映射无需改变 |
| Kimi K3.1 Pro | 上下文 1048576，最大 completion 1048576（默认 131072），推理 low/high/max、默认 max；temperature=1、top_p=.95、n=1、两种 penalty=0；K3 不使用 K2 的 thinking 参数 | 保留长度和推理档位，补齐固定采样描述；native Kimi、SN、OpenRouter 同步移除固定参数和 K2 thinking，让上游使用官方固定默认值。SN/OpenRouter 943718 输出限制不改 |
| GLM-5.3-FlashX（及同源 Flash） | 总参数 320B、激活 18B；上下文 1M、输出 128K；必须开启思考；low/high/max，默认 max | 修正 parameter_scale=320B，补齐激活规模和 thinking_required；已有 native GLM、SN、OpenRouter 操作/推理 lowering 保留。Coding Plan 尚未开放 FlashX，不据此宣称所有 GLM 渠道可调用 |
| Grok 4.7 | 上下文 500000；官方明确无文本输出硬上限；low/medium/high/xhigh、默认 high；temperature 推荐 .7，API 接受 0–2，实际采样不高于 1；top_p 推荐 .95、实际采样不高于 .95 | 原厂不填写人为输出硬上限，补齐采样范围；128000 是默认输出预算，不能写成原厂最大输出。SN/OpenRouter 的 450000 仍仅属于渠道配置 |

来源：

- [OpenAI Flare](https://developers.openai.com/api/docs/models/gpt-image-2.5-flare)、[Sunburst](https://developers.openai.com/api/docs/models/gpt-image-2.5-sunburst)、[图片参数指南](https://developers.openai.com/api/docs/guides/image-prompting)、[Generate API](https://developers.openai.com/api/reference/resources/images/methods/generate)。
- [Kimi K3.1 Pro 快速开始](https://platform.kimi.com/docs/guide/kimi-k3-1-pro-quickstart)、[模型参数比较](https://platform.kimi.com/docs/api/models-overview)、[Chat API](https://platform.kimi.com/docs/api/chat)。
- [GLM Flash/FlashX 原厂说明](https://docs.bigmodel.cn/cn/guide/models/vlm/glm-5.3-flash)、[Z.ai 对应说明](https://docs.z.ai/guides/vlm/glm-5.3-flash)。
- [xAI Grok 4.7](https://docs.x.ai/developers/grok-4-7)、[采样与输出预算说明](https://docs.x.ai/developers/model-capabilities/text/best-practices)。

图片与采样参数的新增 capability 子树是可展示、可查询的技术描述，不是新增参数校验器。现有请求参数模型不能完整表达 background/compression 等所有原厂选项；本轮未扩展公共请求协议，也未承诺这些选项已端到端适配。T1.5 默认成功请求不等于所有尺寸边界及质量档位均已实测。

检查了引用这些模型的 openai、kimi、glm、openrouter、sn provider：native OpenAI 和 GLM 没有新增渠道收窄项，配置保持；Kimi/SN/OpenRouter 更新固定参数规则和相应严格 Mock。T2 baseline 保留每个有效 API 的覆盖，FlashX 保留独立 T2 单元（已撤销依据不足的 Flash 别名去重），本轮没有真实调用。

受影响手册 `aigc_image`、`aigc_audio`、`aigc_video` 已检查：本轮没有改变 CLI、任务结果结构、下载入口或手册命令，故不修改。没有修改公共运行组件、共享类型、存储或依赖；只更新 metadata、测试及文档。

测试报告仅在本地保存，不随源码提交。
