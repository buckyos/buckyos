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
doubao|doubao-responses|llm|responses.create
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
fal|fal-queue|video.extend|queue.submit
fal|fal-queue|video.img2video|queue.submit
fal|fal-queue|video.txt2video|queue.submit
fal|fal-queue|video.upscale|queue.submit
fal|fal-queue|video.video2video|queue.submit
gemini|gemini-interactions|audio.asr|interactions.create
gemini|gemini-interactions|audio.music|interactions.create
gemini|gemini-interactions|audio.tts|interactions.create
gemini|gemini-interactions|embedding.multimodal|models.embedContent
gemini|gemini-interactions|embedding.text|models.embedContent
gemini|gemini-interactions|image.img2img|interactions.create
gemini|gemini-interactions|image.txt2img|interactions.create
gemini|gemini-interactions|llm|interactions.create
gemini|gemini-interactions|video.extend|models.predictLongRunning
gemini|gemini-interactions|video.img2video|models.predictLongRunning
gemini|gemini-interactions|video.txt2video|models.predictLongRunning
gemini|gemini-interactions|video.video2video|models.predictLongRunning
gemini|gemini-interactions|vision.caption|interactions.create
gemini|gemini-interactions|vision.detect|interactions.create
gemini|gemini-interactions|vision.ocr|interactions.create
gemini|gemini-interactions|vision.segment|interactions.create
glm|glm-chat|audio.asr|audio.transcriptions
glm|glm-chat|audio.tts|audio.speech
glm|glm-chat|embedding.text|embeddings.create
glm|glm-chat|image.txt2img|images.generate
glm|glm-chat|llm|chat.completions.create
glm|glm-chat|video.img2video|videos.generate
glm|glm-chat|video.txt2video|videos.generate
glm|glm-chat|vision.caption|chat.completions.create
glm|glm-chat|vision.ocr|chat.completions.create
kimi|kimi-chat|llm|chat.completions.create
kimi|kimi-chat|vision.caption|chat.completions.create
kimi|kimi-chat|vision.ocr|chat.completions.create
minimax|minimax-messages|audio.music|music_generation.create
minimax|minimax-messages|audio.tts|t2a.create
minimax|minimax-messages|image.img2img|image_generation.create
minimax|minimax-messages|image.txt2img|image_generation.create
minimax|minimax-messages|llm|messages.create
minimax|minimax-messages|video.img2video|video_generation.create
minimax|minimax-messages|video.txt2video|video_generation.create
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
openai|openai-responses|video.img2video|videos.create
openai|openai-responses|video.txt2video|videos.create
openai|openai-responses|vision.caption|responses.create
openai|openai-responses|vision.ocr|responses.create
openrouter|openrouter-responses|embedding.text|embeddings.create
openrouter|openrouter-responses|llm|responses.create
openrouter|openrouter-responses|rerank|rerank.create
qwen|qwen-responses|llm|responses.create
sn|sn-openai|llm|responses.create
<!-- END GENERATED BINDINGS -->

运行时的最终选择还取决于具体模型匹配到的 Provider Rules。本文档只描述所有可执行绑定，
不表示每个模型都支持这里列出的全部 API type。
