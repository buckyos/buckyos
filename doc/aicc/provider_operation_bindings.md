# AICC 内置 Provider 执行绑定

本文档列出内置 Provider 的 `provider × adapter × api_type × operation` 完整绑定。
内容由 metadata 与 Codec Registry 的同一 golden 测试校验；修改 Adapter 或 Provider Rules 时必须同步更新。

<!-- BEGIN GENERATED BINDINGS -->
claude|claude-messages|llm|messages.create
claude|claude-messages|vision.caption|messages.create
claude|claude-messages|vision.ocr|messages.create
deepseek|deepseek-responses|llm|responses.create
deepseek|deepseek-responses|vision.caption|responses.create
deepseek|deepseek-responses|vision.ocr|responses.create
doubao-agent-plan|doubao-responses|audio.tts|tts.unidirectional
doubao-agent-plan|doubao-responses|embedding.multimodal|ark.embeddings.multimodal
doubao-agent-plan|doubao-responses|image.img2img|ark.images.generate
doubao-agent-plan|doubao-responses|image.txt2img|ark.images.generate
doubao-agent-plan|doubao-responses|llm|responses.create
doubao-agent-plan|doubao-responses|video.edit|ark.contents.generate
doubao-agent-plan|doubao-responses|video.extend|ark.contents.generate
doubao-agent-plan|doubao-responses|video.generate|ark.contents.generate
doubao-agent-plan|doubao-responses|vision.caption|responses.create
doubao-agent-plan|doubao-responses|vision.ocr|responses.create
doubao-speech|doubao-responses|audio.asr|asr.recognize.flash
doubao-speech|doubao-responses|audio.asr|asr.recognize.submit
doubao-speech|doubao-responses|audio.tts|tts.unidirectional
doubao|doubao-responses|embedding.multimodal|ark.embeddings.multimodal
doubao|doubao-responses|image.img2img|ark.images.generate
doubao|doubao-responses|image.txt2img|ark.images.generate
doubao|doubao-responses|llm|responses.create
doubao|doubao-responses|video.edit|ark.contents.generate
doubao|doubao-responses|video.extend|ark.contents.generate
doubao|doubao-responses|video.generate|ark.contents.generate
doubao|doubao-responses|vision.caption|responses.create
doubao|doubao-responses|vision.ocr|responses.create
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
gemini|gemini-interactions|audio.asr|interactions.create
gemini|gemini-interactions|audio.music|interactions.create
gemini|gemini-interactions|audio.tts|interactions.create
gemini|gemini-interactions|embedding.multimodal|models.embedContent
gemini|gemini-interactions|embedding.text|models.embedContent
gemini|gemini-interactions|image.img2img|interactions.create
gemini|gemini-interactions|image.txt2img|interactions.create
gemini|gemini-interactions|llm|interactions.create
gemini|gemini-interactions|video.edit|interactions.create
gemini|gemini-interactions|video.extend|interactions.create
gemini|gemini-interactions|video.extend|models.predictLongRunning
gemini|gemini-interactions|video.generate|interactions.create
gemini|gemini-interactions|video.generate|models.predictLongRunning
gemini|gemini-interactions|vision.caption|interactions.create
gemini|gemini-interactions|vision.detect|interactions.create
gemini|gemini-interactions|vision.ocr|interactions.create
gemini|gemini-interactions|vision.segment|interactions.create
glm|glm-chat|audio.asr|audio.transcriptions
glm|glm-chat|audio.tts|audio.speech
glm|glm-chat|embedding.text|embeddings.create
glm|glm-chat|image.txt2img|images.generate
glm|glm-chat|llm|chat.completions.create
glm|glm-chat|video.generate|videos.generate
glm|glm-chat|vision.caption|chat.completions.create
glm|glm-chat|vision.ocr|chat.completions.create
kimi|kimi-chat|llm|chat.completions.create
kimi|kimi-chat|vision.caption|chat.completions.create
kimi|kimi-chat|vision.ocr|chat.completions.create
minimax|minimax-messages|audio.asr|speech_to_text.create
minimax|minimax-messages|audio.music|music_generation.create
minimax|minimax-messages|audio.tts|t2a.create
minimax|minimax-messages|image.img2img|image_generation.create
minimax|minimax-messages|image.txt2img|image_generation.create
minimax|minimax-messages|llm|messages.create
minimax|minimax-messages|video.generate|video_generation.create
minimax|minimax-messages|video.generate|video_generation.v2.create
minimax|minimax-messages|vision.caption|messages.create
minimax|minimax-messages|vision.ocr|messages.create
openai|openai-responses|agent.computer_use|responses.create
openai|openai-responses|audio.asr|audio.transcriptions
openai|openai-responses|audio.tts|audio.speech
openai|openai-responses|embedding.text|embeddings.create
openai|openai-responses|image.img2img|images.edit
openai|openai-responses|image.img2img|responses.create
openai|openai-responses|image.inpaint|images.edit
openai|openai-responses|image.txt2img|images.generate
openai|openai-responses|image.txt2img|responses.create
openai|openai-responses|llm|responses.create
openai|openai-responses|video.edit|videos.create
openai|openai-responses|video.generate|videos.create
openai|openai-responses|vision.caption|responses.create
openai|openai-responses|vision.ocr|responses.create
openrouter|openrouter-responses|audio.asr|audio.transcriptions
openrouter|openrouter-responses|audio.music|chat.completions.create
openrouter|openrouter-responses|audio.tts|audio.speech
openrouter|openrouter-responses|decision|decisions.create
openrouter|openrouter-responses|embedding.text|embeddings.create
openrouter|openrouter-responses|image.img2img|images.generate
openrouter|openrouter-responses|image.txt2img|images.generate
openrouter|openrouter-responses|llm|responses.create
openrouter|openrouter-responses|rerank|rerank.create
openrouter|openrouter-responses|video.generate|videos.create
openrouter|openrouter-responses|vision.caption|responses.create
openrouter|openrouter-responses|vision.ocr|responses.create
qwen|qwen-responses|image.img2img|dashscope.image_edit
qwen|qwen-responses|image.img2img|dashscope.image_generation
qwen|qwen-responses|image.txt2img|dashscope.image_generation
qwen|qwen-responses|image.txt2img|dashscope.image_synthesis
qwen|qwen-responses|llm|chat.completions.create
qwen|qwen-responses|llm|responses.create
qwen|qwen-responses|video.generate|dashscope.video_synthesis
qwen|qwen-responses|vision.caption|chat.completions.create
qwen|qwen-responses|vision.caption|responses.create
qwen|qwen-responses|vision.ocr|chat.completions.create
qwen|qwen-responses|vision.ocr|responses.create
sn|sn-openai|agent.computer_use|responses.create
sn|sn-openai|audio.asr|audio.transcriptions
sn|sn-openai|audio.tts|audio.speech
sn|sn-openai|embedding.text|embeddings.create
sn|sn-openai|image.img2img|images.edit
sn|sn-openai|image.img2img|responses.background
sn|sn-openai|image.inpaint|images.edit
sn|sn-openai|image.txt2img|images.generate
sn|sn-openai|image.txt2img|responses.background
sn|sn-openai|llm|responses.create
sn|sn-openai|vision.caption|responses.create
sn|sn-openai|vision.ocr|responses.background
sn|sn-openai|vision.ocr|responses.create
typesafe|typesafe-systemone|decision|systemone.evaluate
<!-- END GENERATED BINDINGS -->

Qwen 的 `responses.create` 委托 `openai-responses` 完成输出解码：若 provider 返回 `image_generation_call.result` 内联数据，基础 codec 必须先将其规范化为结构化 `ResourceRef`，Qwen 层只把 ProviderState namespace 改为 `qwen`。结果物化后公开 output 与 ProviderState 中不得残留 base64，回放时再恢复 Qwen/OpenAI-compatible wire 字符串。`qwen-image-3*` 的文生图和图生图都绑定 `dashscope.image_generation`，以 `X-DashScope-Async: enable` 提交到 `/api/v1/services/aigc/image-generation/generation`，再通过 `/api/v1/tasks/{task_id}` 查询状态和结果；官方协议未声明取消端点，因此该 operation 不声明 cancel。旧版 Qwen 与 Wan 的 `dashscope.image_synthesis`、`dashscope.image_edit` 绑定保持独立。DashScope 图片和视频的官方结果是 URL，作为 URL artifact 保留，不属于内联数据泄漏。

运行时的最终选择还取决于具体模型匹配到的 Provider Rules。本文档只描述所有可执行绑定，
不表示每个模型都支持这里列出的全部 API type。

SN 的 GPT OCR 和 Responses 生图由 `sn.provider.json` 选择 `responses.background`。该 operation 仅声明 NativeTask，提交使用 `POST responses` 与 `background=true`，状态和结果使用 `GET responses/{id}`，取消使用 `POST responses/{id}/cancel`。公开 immediate 请求仍返回统一的 BuckyOS task，不公开 Provider 原生执行模式；缺少上游异步能力时不切换同步接口。OpenAI 直连及其他派生 Provider 的 operation 配置不变。

OpenAI computer-use 强制使用声明的 computer tool。首轮只请求 screenshot 时，Adapter 可将本次已授权的输入截图作为关联 `call_id` 的 `computer_call_output` 回传，并用 `previous_response_id` 续轮一次；不执行业务动作，不自动确认安全检查，不放宽调用方动作白名单。两轮 usage 合并后归入原 task。HTTP future 中断时，未绑定远端任务的本地执行进入结果未知的失败终态，不自动重发；已绑定原生任务或已暂存结果沿用持久生命周期恢复。

公共内部执行端口变更的背景与必要性：单次 `encode/decode` 无法完成官方 computer-use 的截图握手，故增加默认不启用的 codec continuation hook，协议细节仍留在 OpenAI codec；无需让通用执行器识别厂商或模型名称。Gateway 断开导致 future 被丢弃时，任务可能停在已提交状态且失去执行者，这属于所有 Provider 共用的执行生命周期不变量，因此由执行 lease 统一收敛，不能只在 SN codec 修补。两处均不改变公共 kRPC、SDK DTO、资源权限或模型原厂参数。
