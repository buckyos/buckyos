

# AICC 最小 Provider 实现集合

版本：`v0.2-cloud-direct-minimal`
基线：不使用本地模型；不把 OpenRouter 作为 P0 必需 provider。
目标：用最小 provider 集合覆盖主流模型提供方，并覆盖 `agent.computer_use` 之外的全部 AICC ApiType。

## 1. 背景与边界

当前原 Plan 的目标是让 `agent` 之外的 capability namespace 至少有一个可用 provider；现有实现中，Claude、OpenAI、Google Gemini、MiniMax 主要只声明了 `llm.chat`，OpenAI / Gemini 额外声明了 `image.txt2img`，其余 embedding、rerank、vision、audio、video 等 method 仍未接通。原 Plan 中大量最小落地路径依赖本地模型，但本版方案明确不使用本地模型。

本版最小集合采用：

```text
P0 required:
  openai.rs
  claude.rs
  google.rs
  fal.rs

P1 optional:
  openrouter.rs
```

`openrouter.rs` 不进入 P0，因为它与 OpenAI、Claude、Google 的直连能力高度重叠；它只作为长尾模型、成本 fallback、临时 rerank 或模型试用入口。

`agent.computer_use` 不纳入本版 provider coverage。该能力涉及浏览器/桌面 runtime、沙箱、权限和审计，应单独规划。

`llm.completion` 作为 legacy ApiType，不要求 provider-native 实现；系统层统一转换为 `llm.chat` 请求。

---

## 2. P0 最小 Provider 集合

| Provider Adapter | Credential                    | 定位                                             | 必须支持的 ApiType                                                                                                                                                                                                                                      |
| ---------------- | ----------------------------- | ---------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `openai.rs`      | `OPENAI_API_KEY`              | 文本、embedding、图像编辑、ASR/TTS、rerank MVP           | `llm.chat`、`llm.completion` wrapper、`embedding.text`、`rerank`、`image.txt2img`、`image.img2img`、`image.inpaint`、`audio.asr`、`audio.tts`                                                                                                              |
| `claude.rs`      | `ANTHROPIC_API_KEY`           | 主流 LLM 与视觉理解 fallback                          | `llm.chat`、`vision.caption`、`vision.ocr`                                                                                                                                                                                                           |
| `google.rs`      | `GOOGLE_API_KEY` / Gemini key | 多模态主力 provider，覆盖 embedding、vision、music、video | `llm.chat`、`embedding.text`、`embedding.multimodal`、`image.txt2img`、`image.img2img`、`vision.ocr`、`vision.caption`、`vision.detect`、`vision.segment`、`audio.tts`、`audio.music`、`video.txt2video`、`video.img2video`、`video.video2video`、`video.extend` |
| `fal.rs`         | `FAL_KEY`                     | 专用媒体处理工具 provider                              | `image.upscale`、`image.bg_remove`、`audio.enhance`、`video.upscale`                                                                                                                                                                                  |

OpenAI 负责 `embedding.text`、图像生成/编辑、mask inpaint、ASR、TTS 等通用能力；OpenAI 文档中已有 `text-embedding-3-*` embedding 示例，GPT Image 支持生成和编辑图片，image edit 也支持 mask，Audio API 支持 speech-to-text 和 text-to-speech。([OpenAI开发者][1])

Google 是本方案的多模态主力：Gemini Embedding 2 支持 text、image、video、audio、document 映射到同一 embedding space；Gemini 图像能力支持 text-to-image 和 text+image-to-image 编辑；Gemini image understanding 支持 bounding boxes 和 segmentation 示例；Gemini API 也支持 TTS、Lyria 3 音乐生成、Veo 3.1 视频生成、图生视频、video-to-video 和 extend。([Google AI for Developers][2])

Claude 在 P0 中主要用于 `llm.chat` 和视觉理解 fallback。Anthropic 文档说明 Claude vision 可以理解和分析图像，并可通过 API 请求使用。([Claude平台][3])

fal.ai 只用于通用大模型不擅长的专用媒体处理：ESRGAN 负责 image upscale，rembg 负责 background removal，DeepFilterNet3 负责音频降噪/增强，video-upscaler 负责已有视频超分。([Fal.ai][4])

---

## 3. ApiType 覆盖矩阵

| Capability  | ApiType                | 主 Provider               | Fallback / 备注                                              |
| ----------- | ---------------------- | ------------------------ | ---------------------------------------------------------- |
| `llm`       | `llm.chat`             | OpenAI / Claude / Google | 三家都必须声明                                                    |
| `llm`       | `llm.completion`       | System wrapper           | 不做 provider-native；转换为 `llm.chat`                          |
| `embedding` | `embedding.text`       | Google / OpenAI          | Google 可作为主路由，OpenAI fallback                              |
| `embedding` | `embedding.multimodal` | Google                   | 使用 Gemini Embedding 2                                      |
| `rerank`    | `rerank`               | OpenAI 或 Google          | MVP 用 LLM structured rerank；生产可选 Cohere / OpenRouter       |
| `image`     | `image.txt2img`        | Google / OpenAI          | 两家都可声明                                                     |
| `image`     | `image.img2img`        | Google / OpenAI          | Google image edit 或 OpenAI image edit                      |
| `image`     | `image.inpaint`        | OpenAI                   | 使用 image edit + mask；Google 可做 prompt-guided edit fallback |
| `image`     | `image.upscale`        | fal.ai                   | `fal-ai/esrgan`                                            |
| `image`     | `image.bg_remove`      | fal.ai                   | `fal-ai/imageutils/rembg`                                  |
| `vision`    | `vision.ocr`           | Google                   | Claude / OpenAI vision 可 fallback                          |
| `vision`    | `vision.caption`       | Google / Claude          | Claude 适合通用视觉理解 fallback                                   |
| `vision`    | `vision.detect`        | Google                   | 使用 bounding box / spatial understanding                    |
| `vision`    | `vision.segment`       | Google                   | 使用 segmentation masks 能力                                   |
| `audio`     | `audio.asr`            | OpenAI                   | `audio/transcriptions`                                     |
| `audio`     | `audio.tts`            | OpenAI / Google          | OpenAI speech 或 Gemini TTS                                 |
| `audio`     | `audio.music`          | Google                   | Lyria 3                                                    |
| `audio`     | `audio.enhance`        | fal.ai                   | `fal-ai/deepfilternet3`                                    |
| `video`     | `video.txt2video`      | Google                   | Veo 3.1                                                    |
| `video`     | `video.img2video`      | Google                   | Veo 3.1 image-to-video                                     |
| `video`     | `video.video2video`    | Google                   | Veo 3.1 支持 video-to-video 输入模式                             |
| `video`     | `video.extend`         | Google                   | Veo 3.1 extend；需要保留 `continuation_handle`                  |
| `video`     | `video.upscale`        | fal.ai                   | `fal-ai/video-upscaler`                                    |

---

## 4. Provider 必须实现的最小方法清单

### 4.1 `openai.rs`

必须实现：

```text
llm.chat
llm.completion        # system-level wrapper to chat
embedding.text
rerank                # MVP: LLM structured rerank
image.txt2img
image.img2img
image.inpaint
audio.asr
audio.tts
```

说明：

* `llm.completion` 不接旧 Completion API，统一转成 chat message。
* `rerank` 是 MVP 实现，使用 LLM 对候选文档输出 structured scores；不是 native rerank model。
* `image.inpaint` 使用 OpenAI image edit + mask。
* 不要求 OpenAI 承担视频主路由。

---

### 4.2 `claude.rs`

必须实现：

```text
llm.chat
vision.caption
vision.ocr
```

说明：

* `vision.ocr` 是“视觉文本提取”语义，不要求传统 OCR engine 的逐字坐标级输出。
* Claude 不作为 `vision.detect` / `vision.segment` 主路由。
* Claude 不承担 embedding、image generation、audio、video。

---

### 4.3 `google.rs`

必须实现：

```text
llm.chat

embedding.text
embedding.multimodal

image.txt2img
image.img2img

vision.ocr
vision.caption
vision.detect
vision.segment

audio.tts
audio.music

video.txt2video
video.img2video
video.video2video
video.extend
```

说明：

* Google 是本版的多模态主 provider。
* `embedding.multimodal` 必须记录 `embedding_space_id`，并禁止与 OpenAI embedding 混用。
* `video.extend` 必须保存 `continuation_handle`，并遵守“视频任务 Started 后不跨 provider 重试”的规则。
* `video.video2video` 使用 Veo 3.1 的 video input / extension / video-to-video 能力；如果后续需要更复杂的视频风格迁移，可作为 P1 再接专门 video provider。

---

### 4.4 `fal.rs`

必须实现：

```text
image.upscale
image.bg_remove
audio.enhance
video.upscale
```

说明：

* fal.ai 不作为 LLM provider。
* fal.ai 不承担主流 LLM、embedding、rerank、ASR、TTS、music、video generation 的主路由。
* fal.ai 只补工具型媒体处理能力。
* 所有 fal 视频类任务必须走 task-manager 异步路径。

推荐 exact model：

```text
image.upscale   -> fal-ai/esrgan@fal
image.bg_remove -> fal-ai/imageutils/rembg@fal
audio.enhance   -> fal-ai/deepfilternet3@fal
video.upscale   -> fal-ai/video-upscaler@fal
```

---

## 5. OpenRouter 的位置

`openrouter.rs` 不进入最小 P0 集合。

它可以作为 P1 optional provider，用于：

```text
openrouter.rs
  + long-tail llm.chat       # DeepSeek / Qwen / Mistral / Llama / xAI 等
  + rerank                   # 若不想直连 Cohere
  + cost fallback
  + model discovery
```

不建议用 OpenRouter 覆盖：

```text
embedding.text              # OpenAI / Google 已覆盖
embedding.multimodal        # Google 更直接
image.*                     # OpenAI / Google / fal 更直接
vision.*                    # Google / Claude 更直接
audio.*                     # OpenAI / Google / fal 更直接
video.*                     # Google / fal 更直接
```

---

## 6. 最小实现结论

最终 P0 provider 集合为：

```text
openai.rs
claude.rs
google.rs
fal.rs
```

其中真正新增 key 只有：

```text
FAL_KEY
```

已有的：

```text
OPENAI_API_KEY
ANTHROPIC_API_KEY
GOOGLE_API_KEY
```

继续作为直连主 provider 使用。

该集合覆盖 `agent.computer_use` 之外的全部 AICC ApiType；`OpenRouter`、`Cohere`、`MiniMax`、`Kling`、`Runway`、`ByteDance` 等均不作为 P0 必需 provider。

[1]: https://developers.openai.com/api/docs/guides/embeddings "Vector embeddings | OpenAI API"
[2]: https://ai.google.dev/gemini-api/docs/embeddings "Embeddings  |  Gemini API  |  Google AI for Developers"
[3]: https://platform.claude.com/docs/en/build-with-claude/vision "Vision - Claude API Docs"
[4]: https://fal.ai/models/fal-ai/esrgan/api "Upscale Images | Image to Image | fal.ai"
