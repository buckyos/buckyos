# AICC 内置 Provider 执行绑定

本文档列出内置 Provider 的 `provider × adapter × api_type × operation` 完整绑定。
内容由 metadata 与 Codec Registry 的同一 golden 测试校验；修改 Adapter 或 Provider Rules 时必须同步更新。

<!-- BEGIN GENERATED BINDINGS -->
bailian|bailian-media|image.img2img|dashscope.image_edit
bailian|bailian-media|image.img2img|dashscope.image_generation
bailian|bailian-media|image.txt2img|dashscope.image_generation
bailian|bailian-media|image.txt2img|dashscope.image_synthesis
bailian|bailian-media|video.generate|dashscope.video_synthesis
bailian|bailian-responses|llm|responses.create
bailian|bailian-responses|vision.caption|responses.create
bailian|bailian-responses|vision.ocr|responses.create
bailian|legacy-openai-chat-completions|llm|chat.completions.create
bailian|legacy-openai-chat-completions|vision.caption|chat.completions.create
bailian|legacy-openai-chat-completions|vision.ocr|chat.completions.create
claude|claude-messages|llm|messages.create
claude|claude-messages|vision.caption|messages.create
claude|claude-messages|vision.ocr|messages.create
deepseek|openai-responses|llm|responses.create
deepseek|openai-responses|vision.caption|responses.create
deepseek|openai-responses|vision.ocr|responses.create
doubao-agent-plan|doubao-media|embedding.multimodal|ark.embeddings.multimodal
doubao-agent-plan|doubao-media|image.img2img|ark.images.generate
doubao-agent-plan|doubao-media|image.txt2img|ark.images.generate
doubao-agent-plan|doubao-media|video.edit|ark.contents.generate
doubao-agent-plan|doubao-media|video.extend|ark.contents.generate
doubao-agent-plan|doubao-media|video.generate|ark.contents.generate
doubao-agent-plan|doubao-speech|audio.tts|tts.unidirectional
doubao-agent-plan|openai-responses|llm|responses.create
doubao-agent-plan|openai-responses|vision.caption|responses.create
doubao-agent-plan|openai-responses|vision.ocr|responses.create
doubao-speech|doubao-speech|audio.asr|asr.recognize.flash
doubao-speech|doubao-speech|audio.asr|asr.recognize.idle.submit
doubao-speech|doubao-speech|audio.asr|asr.recognize.submit
doubao-speech|doubao-speech|audio.tts|tts.unidirectional
doubao|doubao-media|embedding.multimodal|ark.embeddings.multimodal
doubao|doubao-media|image.img2img|ark.images.generate
doubao|doubao-media|image.txt2img|ark.images.generate
doubao|doubao-media|video.edit|ark.contents.generate
doubao|doubao-media|video.extend|ark.contents.generate
doubao|doubao-media|video.generate|ark.contents.generate
doubao|openai-responses|llm|responses.create
doubao|openai-responses|vision.caption|responses.create
doubao|openai-responses|vision.ocr|responses.create
fal|fal-queue|audio.asr|queue.submit
fal|fal-queue|audio.enhance|queue.submit
fal|fal-queue|audio.music|queue.submit
fal|fal-queue|audio.tts|queue.submit
fal|fal-queue|image.bg_remove|queue.submit
fal|fal-queue|image.img2img|queue.submit
fal|fal-queue|image.inpaint|queue.submit
fal|fal-queue|image.txt2img|queue.submit
fal|fal-queue|image.upscale|queue.submit
fal|fal-queue|video.upscale|queue.submit
gemini|gemini-generate-content|embedding.multimodal|models.embedContent
gemini|gemini-generate-content|embedding.text|models.embedContent
gemini|gemini-generate-content|video.extend|models.predictLongRunning
gemini|gemini-generate-content|video.generate|models.predictLongRunning
gemini|gemini-interactions|audio.asr|interactions.create
gemini|gemini-interactions|audio.music|interactions.create
gemini|gemini-interactions|audio.tts|interactions.create
gemini|gemini-interactions|image.img2img|interactions.create
gemini|gemini-interactions|image.txt2img|interactions.create
gemini|gemini-interactions|llm|interactions.create
gemini|gemini-interactions|video.edit|interactions.create
gemini|gemini-interactions|video.extend|interactions.create
gemini|gemini-interactions|video.generate|interactions.create
gemini|gemini-interactions|vision.caption|interactions.create
gemini|gemini-interactions|vision.detect|interactions.create
gemini|gemini-interactions|vision.ocr|interactions.create
gemini|gemini-interactions|vision.segment|interactions.create
glm|glm-video|video.generate|videos.generate
glm|legacy-openai-chat-completions|llm|chat.completions.create
glm|legacy-openai-chat-completions|vision.caption|chat.completions.create
glm|legacy-openai-chat-completions|vision.ocr|chat.completions.create
glm|openai-audio|audio.asr|audio.transcriptions
glm|openai-audio|audio.tts|audio.speech
glm|openai-embeddings|embedding.text|embeddings.create
glm|openai-images|image.txt2img|images.generate
kimi|legacy-openai-chat-completions|llm|chat.completions.create
kimi|legacy-openai-chat-completions|vision.caption|chat.completions.create
kimi|legacy-openai-chat-completions|vision.ocr|chat.completions.create
minimax|legacy-claude-messages|llm|messages.create
minimax|legacy-claude-messages|vision.caption|messages.create
minimax|legacy-claude-messages|vision.ocr|messages.create
minimax|minimax-media|audio.asr|speech_to_text.create
minimax|minimax-media|audio.music|music_generation.create
minimax|minimax-media|audio.tts|t2a.create
minimax|minimax-media|image.img2img|image_generation.create
minimax|minimax-media|image.txt2img|image_generation.create
minimax|minimax-media|video.generate|video_generation.create
minimax|minimax-media|video.generate|video_generation.v2.create
openai|openai-audio|audio.asr|audio.transcriptions
openai|openai-audio|audio.tts|audio.speech
openai|openai-embeddings|embedding.text|embeddings.create
openai|openai-images|image.img2img|images.edit
openai|openai-images|image.inpaint|images.edit
openai|openai-images|image.txt2img|images.generate
openai|openai-responses|agent.computer_use|responses.create
openai|openai-responses|image.img2img|responses.create
openai|openai-responses|image.txt2img|responses.create
openai|openai-responses|llm|responses.create
openai|openai-responses|vision.caption|responses.create
openai|openai-responses|vision.ocr|responses.create
openai|openai-videos|video.edit|videos.create
openai|openai-videos|video.generate|videos.create
openrouter|openai-audio|audio.asr|audio.transcriptions
openrouter|openai-embeddings|embedding.text|embeddings.create
openrouter|openrouter-native|audio.music|chat.completions.create
openrouter|openrouter-native|audio.tts|audio.speech
openrouter|openrouter-native|decision|decisions.create
openrouter|openrouter-native|image.img2img|images.generate
openrouter|openrouter-native|image.txt2img|images.generate
openrouter|openrouter-native|video.generate|videos.create
openrouter|openrouter-responses|llm|responses.create
openrouter|openrouter-responses|vision.caption|responses.create
openrouter|openrouter-responses|vision.ocr|responses.create
openrouter|rerank|rerank|rerank.create
sn|openai-audio|audio.asr|audio.transcriptions
sn|openai-audio|audio.tts|audio.speech
sn|openai-chat-completions|audio.asr|chat.completions.create
sn|openai-embeddings|embedding.text|embeddings.create
sn|openai-images|image.img2img|images.edit
sn|openai-images|image.inpaint|images.edit
sn|openai-images|image.txt2img|images.generate
sn|openai-responses|agent.computer_use|responses.create
sn|openai-responses|image.img2img|responses.background
sn|openai-responses|image.txt2img|responses.background
sn|openai-responses|llm|responses.create
sn|openai-responses|vision.caption|responses.create
sn|openai-responses|vision.ocr|responses.background
sn|openai-responses|vision.ocr|responses.create
sn|openai-videos|video.edit|videos.create
sn|openai-videos|video.extend|videos.create
sn|openai-videos|video.generate|videos.create
typesafe|typesafe-systemone|decision|systemone.evaluate
<!-- END GENERATED BINDINGS -->

百炼的 `responses.create`（`bailian-responses`）委托 `openai-responses` 完成输出解码：若 provider 返回 `image_generation_call.result` 内联数据，基础 codec 必须先将其规范化为结构化 `ResourceRef`；ProviderState 的命名空间 `qwen` 由 Known Provider 的 `state_namespace` 声明，执行层统一改写。结果物化后公开 output 与 ProviderState 中不得残留 base64，回放时再恢复 Qwen/OpenAI-compatible wire 字符串。`qwen-image-3*` 的文生图和图生图都绑定 `dashscope.image_generation`，以 `X-DashScope-Async: enable` 提交到 `/api/v1/services/aigc/image-generation/generation`，再通过 `/api/v1/tasks/{task_id}` 查询状态和结果；官方协议未声明取消端点，因此该 operation 不声明 cancel。旧版 Qwen 与 Wan 的 `dashscope.image_synthesis`、`dashscope.image_edit` 绑定保持独立。DashScope 图片和视频的官方结果是 URL，作为 URL artifact 保留，不属于内联数据泄漏。

运行时的最终选择还取决于具体模型匹配到的 Provider Rules。本文档只描述所有可执行绑定，
不表示每个模型都支持这里列出的全部 API type。

SN 的 GPT OCR 和 Responses 生图由 `sn.provider.json` 选择 `responses.background`。该 operation 仅声明 NativeTask，提交使用 `POST responses` 与 `background=true`，状态和结果使用 `GET responses/{id}`，取消使用 `POST responses/{id}/cancel`。公开 immediate 请求仍返回统一的 BuckyOS task，不公开 Provider 原生执行模式；缺少上游异步能力时不切换同步接口。OpenAI 直连及其他派生 Provider 的 operation 配置不变。

OpenAI computer-use 强制使用声明的 computer tool。首轮只请求 screenshot 时，Adapter 可将本次已授权的输入截图作为关联 `call_id` 的 `computer_call_output` 回传，并用 `previous_response_id` 续轮一次；不执行业务动作，不自动确认安全检查，不放宽调用方动作白名单。两轮 usage 合并后归入原 task。HTTP future 中断时，未绑定远端任务的本地执行进入结果未知的失败终态，不自动重发；已绑定原生任务或已暂存结果沿用持久生命周期恢复。

公共内部执行端口变更的背景与必要性：单次 `encode/decode` 无法完成官方 computer-use 的截图握手，故增加默认不启用的 codec continuation hook，协议细节仍留在 OpenAI codec；无需让通用执行器识别厂商或模型名称。Gateway 断开导致 future 被丢弃时，任务可能停在已提交状态且失去执行者，这属于所有 Provider 共用的执行生命周期不变量，因此由执行 lease 统一收敛，不能只在 SN codec 修补。两处均不改变公共 kRPC、SDK DTO、资源权限或模型原厂参数。

SN 的 Qwen Image 使用 `images.generate` / `images.edit` 公共入口。其 OpenRouter 上游要求独立 `/images` API，不能使用 Chat 图片协议；当前 SN 实测未取得可供轮询的远端图片任务 ID，不能据此判定上游是否支持后台任务。Qwen3.7 OCR 使用 `responses.create`，与其图片描述接口保持一致。SN 使用标准 OpenAI 族客户端（`openai-responses`、`openai-images`、`openai-videos` 等），不再有 SN 私有 adapter。Wan 视频编辑与延长在 SN 规则里是排除的；`openai-videos` 的延长按官方 `POST /videos/extensions` 实现，不带 `model`。

公共执行器修复的必要性：原生任务轮询中的 TaskMgr 进度上报若临时失败，原实现会退出轮询并遗留活动任务。现在可重试的进度上报错误不会中断同一远端任务的轮询，不可重试错误进入失败终态；不重新提交推理。该生命周期约束适用于全部原生任务，必须在公共执行器处理，不能依赖 SN 私有协议修补；未改变公共接口或数据结构。
