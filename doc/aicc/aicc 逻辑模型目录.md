# AICC 模型目录树设计 

本文档是 `aicc_router.md` 的配套设计,定义 AICC 标准化的:

1. **API Type 枚举**:Provider 必须从这个有限集合声明能力,新增需主版本升级。
2. **一级逻辑目录**:用户/Agent 调用 AICC 时使用的 namespace。
3. **二级模型家族目录 + 热门模型映射**:供 `logical_mounts` 和 `items` 软链接使用。

更新基线时间:2026-04-24。

---

## 一、API Type 枚举(必要项)

API Type 决定 request/response schema。**Provider 不能自定义 api_type**,必须从下表选择;同一精确模型可声明多个 api_type。

### 1.1 文本类

| api_type | 输入 | 输出 | 说明 |
|---|---|---|---|
| `llm.chat` | `messages[]`(可含 image/audio block) | `message` + 可选 `tool_calls` | 主流 chat completion,事实标准 |
| `llm.completion` | `prompt: string` | `text: string` | 兼容老接口,新模型不必声明 |
| `embedding.text` | `string \| string[]` | `number[][]` | 文本/代码 embedding |
| `embedding.multimodal` | `text \| image \| (text+image)` | `number[][]` | CLIP 类跨模态 embedding |
| `rerank` | `query, docs[]` | `score[]` | Cross-encoder 重排序 |

### 1.2 图像类

| api_type | 输入 | 输出 | 说明 |
|---|---|---|---|
| `image.txt2img` | `prompt + 参数` | `image` | 文生图 |
| `image.img2img` | `image + prompt + 参数` | `image` | 图生图/编辑 |
| `image.inpaint` | `image + mask + prompt` | `image` | 局部重绘 |
| `image.upscale` | `image + scale_factor` | `image` | 超分 |
| `image.bg_remove` | `image` | `image`(带 alpha) | 抠图 |
| `vision.ocr` | `image` | `[{bbox, text, confidence}]` | OCR(结构化输出) |
| `vision.caption` | `image` | `string` | 图像描述(短文本) |
| `vision.detect` | `image + 可选 classes[]` | `[{bbox, label, score}]` | 目标检测 |
| `vision.segment` | `image + 可选 prompt` | `mask[]` | 分割(SAM 类) |

注:图像**理解**走 `vision.*`(结构化输出);若希望模型返回自由文本(如 VQA),走 `llm.chat` 并在 `messages` 中传 image block。

### 1.3 音频类

| api_type | 输入 | 输出 | 说明 |
|---|---|---|---|
| `audio.tts` | `text + voice_id` | `audio` | 文转语音 |
| `audio.asr` | `audio` | `text + 可选 timestamps` | 语音识别(转录/字幕) |
| `audio.music` | `prompt + duration` | `audio` | 音乐生成 |
| `audio.enhance` | `audio + 任务参数` | `audio` | 降噪、人声分离、混响去除 |

### 1.4 视频类

| api_type | 输入 | 输出 | 说明 |
|---|---|---|---|
| `video.txt2video` | `prompt + 参数` | `video` | 文生视频 |
| `video.img2video` | `image + prompt` | `video` | 图生视频 |
| `video.video2video` | `video + prompt` | `video` | 视频编辑/风格转换 |
| `video.extend` | `video + prompt` | `video` | 视频续写 |
| `video.upscale` | `video + scale` | `video` | 视频超分 |

### 1.5 Agent Runtime Support 类

| api_type | 输入 | 输出 | 说明 |
|---|---|---|---|
| `agent.computer_use` | `screenshot + 任务` | `action[]`(click/type/scroll) | 桌面/浏览器操作 |

这个方向还在高速发展，占位为主
---

## 二、一级逻辑目录与 API Type 对应

每个一级目录是一个 namespace,声明该 namespace 下叶子节点支持的 api_type 集合。**fallback 只能在同 api_type 内进行**。

| 一级目录 | 默认 api_type | fallback 策略 | 默认调度 profile |
|---|---|---|---|
| `llm` | `llm.chat`(主)、`llm.completion` | parent | balanced |
| `embedding` | `embedding.text`、`embedding.multimodal` | **strict**(向量空间不通用) | latency_first |
| `rerank` | `rerank` | strict | latency_first |
| `image` | `image.*`、`vision.*` | parent within same api_type | quality_first |
| `audio` | `audio.*` | strict(音色/语种不能跨) | latency_first(tts/asr)、quality_first(music) |
| `video` | `video.*` | parent within same api_type | quality_first |
| `multimodal` | (any-to-any,占位) | disabled | — |
| `agent_runtime` | `agent.*` | parent | reliability_first |

### 2.1 `llm` 子目录

```
llm
├── chat       # 通用对话(默认入口)
├── plan       # 高质量规划(Agent 用)
├── code       # 代码任务
├── reason     # 显式 reasoning(o1/r1/k2-thinking 类,延迟高)
├── vision     # 需要传图的对话(VLM)
├── swift      # 极速响应(短回复、低延迟)
├── summary    # 总结/抽取(可用便宜模型)
├── translate  # 翻译
├── long       # 超长上下文(>200k)
└── fallback   # 兜底兜底
```

支持的 api_type:`llm.chat`(主)。`llm.completion` 只在历史模型上保留。

### 2.2 `embedding` 子目录

```
embedding
├── text          # 通用文本
├── multilingual  # 多语种
├── code          # 代码
└── multimodal    # 跨模态(CLIP/SigLIP)
```

### 2.3 `rerank` 子目录

```
rerank
├── general       # 通用 rerank
└── multilingual  # 多语种 rerank
```

### 2.4 `image` 子目录

```
image
├── txt2img       # api_type: image.txt2img
├── img2img       # api_type: image.img2img
├── inpaint       # api_type: image.inpaint
├── upscale       # api_type: image.upscale
├── bg_remove     # api_type: image.bg_remove
├── ocr           # api_type: vision.ocr
├── caption       # api_type: vision.caption
├── detect        # api_type: vision.detect
└── segment       # api_type: vision.segment
```

### 2.5 `audio` 子目录

```
audio
├── tts           # api_type: audio.tts
├── asr           # api_type: audio.asr
├── music         # api_type: audio.music
└── enhance       # api_type: audio.enhance
```

### 2.6 `video` 子目录

```
video
├── txt2video     # api_type: video.txt2video
├── img2video     # api_type: video.img2video
├── video2video   # api_type: video.video2video
├── extend        # api_type: video.extend
└── upscale       # api_type: video.upscale
```

### 2.7 `agent_runtime` 子目录

```
agent_runtime
├── computer_use   # api_type: agent.computer_use

```

---

## 三、模型家族目录与热门模型挂载

### 设计约定

- **家族目录**与角色目录、任务目录正交。一个精确模型同时挂载到家族目录 + 任务目录 + (可选)角色目录。
- **闭源模型**通常也声明家族目录(`llm.gpt5`、`llm.claude`),便于 UI 展示和用户偏好表达。
- **同一家族内的不同尺寸/变体**(如 `claude-opus-4.7` vs `claude-haiku-4.5`)是不同的精确模型,通过 `attributes.tier`(flagship / mid / nano)区分。

### 3.1 `llm` 家族目录

```
llm
├── gpt5         # OpenAI GPT-5 系列
├── claude       # Anthropic Claude 4.x
├── gemini       # Google Gemini 3.x
├── grok         # xAI Grok 4.x
├── qwen         # Alibaba Qwen 3.x(开源)
├── llama        # Meta Llama 4.x(开源)
├── deepseek     # DeepSeek V3/V4(开源)
├── kimi         # Moonshot Kimi K2.x(开源)
├── glm          # Zhipu GLM 5.x(开源)
├── minimax      # MiniMax M2.x(开源)
├── mistral      # Mistral 系列(开源 + 云)
├── gemma        # Google Gemma 3(开源)
├── gpt_oss      # OpenAI 开源版(gpt-oss-20b/120b)
└── nemotron     # NVIDIA Nemotron(开源)
```

#### 热门模型挂载示例(2026-04 基线)

| 精确模型 | 家族 | 任务挂载 | 角色推荐 | tier |
|---|---|---|---|---|
| `gpt-5.5@openai` | gpt5 | chat, vision | plan, code | flagship |
| `gpt-5.5-mini@openai` | gpt5 | chat | swift, summary | mid |
| `gpt-5.5-nano@openai` | gpt5 | chat | swift | nano |
| `gpt-5.4@openai` | gpt5 | chat, vision | code | flagship |
| `claude-opus-4.7@anthropic` | claude | chat, vision | plan, code, reason | flagship |
| `claude-opus-4.6@anthropic` | claude | chat, vision | code | flagship |
| `claude-sonnet-4.6@anthropic` | claude | chat, vision | chat, code | mid |
| `claude-haiku-4.5@anthropic` | claude | chat | swift, summary | nano |
| `gemini-3.1-pro@google` | gemini | chat, vision, long | plan, code, long | flagship |
| `gemini-3.1-flash@google` | gemini | chat, vision | chat, swift | mid |
| `gemini-3.1-flash-lite@google` | gemini | chat | swift | nano |
| `grok-4@xai` | grok | chat, reason | reason | flagship |
| `qwen3.5-max@alibaba` | qwen | chat | plan, chat | flagship |
| `qwen3.5-coder-480b@alibaba` | qwen | chat | code | flagship |
| `qwen3.5-9b@local` | qwen | chat | swift, summary | nano |
| `llama4-maverick@meta` | llama | chat, vision | chat | mid |
| `llama4-scout@meta` | llama | chat, long | long | mid |
| `deepseek-v4-pro@deepseek` | deepseek | chat, reason | plan, reason | flagship |
| `deepseek-v4-flash@deepseek` | deepseek | chat | summary, code | mid |
| `kimi-k2.5@moonshot` | kimi | chat | code, plan | flagship |
| `kimi-k2-thinking@moonshot` | kimi | chat, reason | reason | flagship |
| `glm-5.1@zhipu` | glm | chat | code | flagship |
| `minimax-m2.5@minimax` | minimax | chat | code | flagship |
| `gpt-oss-120b@local` | gpt_oss | chat | chat(本地优先场景) | mid |
| `gemma-3-27b@local` | gemma | chat | swift | mid |

**Reasoning 角色专属**:`llm.reason` 默认只挂载支持 thinking 输出的模型 —— `claude-opus-4.7` (with extended thinking)、`grok-4`、`deepseek-v4-pro`、`kimi-k2-thinking`、`gpt-5.5` (xhigh)。

**Vision 角色**:`llm.vision` 挂载所有 VLM,排除纯文本模型(deepseek-v4-pro 当前为纯文本)。

### 3.2 `embedding` 家族目录

```
embedding
├── openai         # text-embedding-3-{small,large}
├── voyage         # voyage-3, voyage-code-3
├── cohere         # cohere-embed-v4
├── jina           # jina-embeddings-v4, jina-clip-v2
├── bge            # bge-m3, bge-large(开源)
├── e5             # multilingual-e5-large(开源)
├── nomic          # nomic-embed-text-v2(开源)
├── granite        # granite-embedding(IBM 开源)
└── qwen_embed     # qwen3-embedding(开源)
```

#### 热门挂载

| 精确模型 | 家族 | 任务挂载 |
|---|---|---|
| `text-embedding-3-large@openai` | openai | embedding.text |
| `voyage-3@voyageai` | voyage | embedding.text、embedding.multilingual |
| `voyage-code-3@voyageai` | voyage | embedding.code |
| `cohere-embed-v4@cohere` | cohere | embedding.text、embedding.multilingual |
| `jina-embeddings-v4@jina` | jina | embedding.text、embedding.code |
| `jina-clip-v2@jina` | jina | embedding.multimodal |
| `bge-m3@local` | bge | embedding.text、embedding.multilingual |
| `qwen3-embedding-8b@local` | qwen_embed | embedding.text、embedding.multilingual |

### 3.3 `rerank` 家族目录

```
rerank
├── cohere         # cohere-rerank-v3.5
├── voyage         # voyage-rerank-2
├── jina           # jina-reranker-v3
└── bge_rerank     # bge-reranker-v2-m3(开源)
```

### 3.4 `image` 家族目录

```
image
├── flux           # Black Forest Labs FLUX 系列
├── flux_kontext   # FLUX Kontext(图像编辑专用)
├── seedream       # ByteDance Seedream
├── imagen         # Google Imagen
├── gpt_image      # OpenAI gpt-image-1 / dall-e
├── ideogram       # Ideogram(强文本渲染)
├── recraft        # Recraft(SVG/设计向)
├── qwen_image     # Alibaba Qwen-Image
├── sd             # Stable Diffusion 系列(SDXL、SD 3.5)
├── topaz          # Topaz(超分专精)
├── real_esrgan    # Real-ESRGAN(开源超分)
├── codeformer     # CodeFormer(人脸修复)
└── sam            # Segment Anything(分割)
```

#### 热门挂载

| 精确模型 | 家族 | 任务挂载 |
|---|---|---|
| `flux-1.1-pro@bfl` | flux | image.txt2img |
| `flux-dev@local` | flux | image.txt2img |
| `flux-schnell@local` | flux | image.txt2img(swift 场景) |
| `flux-kontext-pro@bfl` | flux_kontext | image.img2img、image.inpaint |
| `flux-fill@bfl` | flux | image.inpaint |
| `seedream-5.0@bytedance` | seedream | image.txt2img、image.img2img |
| `imagen-4@google` | imagen | image.txt2img |
| `gpt-image-1@openai` | gpt_image | image.txt2img、image.img2img |
| `ideogram-v3@ideogram` | ideogram | image.txt2img(海报、文字向) |
| `recraft-v4@recraft` | recraft | image.txt2img(SVG) |
| `qwen-image-2-pro@alibaba` | qwen_image | image.txt2img、image.img2img |
| `sdxl@local` | sd | image.txt2img |
| `sd-3.5-large@local` | sd | image.txt2img |
| `topaz-image-upscale@topaz` | topaz | image.upscale |
| `real-esrgan@local` | real_esrgan | image.upscale |
| `gfpgan@local` | codeformer | image.img2img(人脸) |
| `sam-2@meta` | sam | vision.segment |
| `florence-2@microsoft` | — | vision.ocr、vision.caption、vision.detect |
| `paddleocr@local` | — | vision.ocr |
| `rmbg-2@bria` | — | image.bg_remove |

### 3.5 `audio` 家族目录

```
audio
├── elevenlabs     # ElevenLabs(高质量 TTS)
├── openai_audio   # OpenAI TTS / Whisper
├── kokoro         # Kokoro TTS(开源轻量)
├── fish_speech    # Fish Speech(开源)
├── whisper        # Whisper 系列(开源 ASR)
├── parakeet       # NVIDIA Parakeet(开源 ASR)
├── sensevoice     # Alibaba SenseVoice(开源 ASR,多语种)
├── suno           # Suno(音乐)
├── udio           # Udio(音乐)
├── lyria          # Google Lyria(音乐)
├── musicgen       # Meta MusicGen(开源音乐)
├── stable_audio   # Stable Audio(开源音乐)
└── demucs         # Demucs(音源分离)
```

#### 热门挂载

| 精确模型 | 家族 | 任务挂载 |
|---|---|---|
| `eleven-v3@elevenlabs` | elevenlabs | audio.tts |
| `tts-1-hd@openai` | openai_audio | audio.tts |
| `kokoro-82m@local` | kokoro | audio.tts(本地 swift) |
| `whisper-large-v3@local` | whisper | audio.asr |
| `whisper-1@openai` | whisper | audio.asr |
| `parakeet-tdt@local` | parakeet | audio.asr(英文低延迟) |
| `sensevoice-small@local` | sensevoice | audio.asr(多语种) |
| `suno-v5@suno` | suno | audio.music |
| `lyria-3-pro@google` | lyria | audio.music |
| `musicgen-large@local` | musicgen | audio.music |
| `demucs-v4@local` | demucs | audio.enhance |

### 3.6 `video` 家族目录

```
video
├── seedance       # ByteDance Seedance
├── kling          # Kling(快手)
├── wan            # Alibaba Wan
├── sora           # OpenAI Sora
├── veo            # Google Veo / Lyria
├── pixverse       # PixVerse
├── grok_imagine   # xAI Grok Imagine Video
├── hunyuan_video  # Tencent Hunyuan Video(开源)
├── mochi          # Genmo Mochi(开源)
└── topaz_video    # Topaz Video(超分)
```

#### 热门挂载

| 精确模型 | 家族 | 任务挂载 |
|---|---|---|
| `seedance-2.0@bytedance` | seedance | video.txt2video、video.img2video |
| `kling-3.0@kling` | kling | video.txt2video、video.img2video |
| `kling-3.0-omni@kling` | kling | video.video2video |
| `wan-2.7@alibaba` | wan | video.img2video |
| `sora-2@openai` | sora | video.txt2video |
| `veo-3@google` | veo | video.txt2video |
| `pixverse-v5@pixverse` | pixverse | video.txt2video |
| `grok-imagine-video@xai` | grok_imagine | video.extend |
| `hunyuan-video@local` | hunyuan_video | video.txt2video |
| `topaz-video-upscale@topaz` | topaz_video | video.upscale |


---

## 四、配置示例:一份完整 global session config

把上面的目录树落到 `aicc_router.md` 第 11 节定义的 SessionConfig schema:

```yaml
session_config:
  schema_version: 1
  revision: 0
  default_profile: balanced
  
  logical_tree:
    # ===== LLM =====
    llm.plan:
      items:
        opus:    { target: llm.claude,   weight: 3.0 }
        gpt5:    { target: llm.gpt5,     weight: 2.5 }
        gemini:  { target: llm.gemini,   weight: 2.0 }
        deepseek:{ target: llm.deepseek, weight: 1.0 }
      fallback: { mode: parent }
      profile: quality_first

    llm.code:
      items:
        opus:     { target: llm.claude,    weight: 3.0 }
        gpt5:     { target: llm.gpt5,      weight: 2.5 }
        gemini:   { target: llm.gemini,    weight: 2.5 }
        kimi:     { target: llm.kimi,      weight: 2.0 }
        glm:      { target: llm.glm,       weight: 1.5 }
        deepseek: { target: llm.deepseek,  weight: 1.5 }
        qwen:     { target: llm.qwen,      weight: 1.0 }
      fallback: { mode: parent }

    llm.swift:
      items:
        haiku:        { target: claude-haiku-4.5@anthropic, weight: 3.0 }
        flash_lite:   { target: gemini-3.1-flash-lite@google, weight: 2.5 }
        gpt5_nano:    { target: gpt-5.5-nano@openai, weight: 2.5 }
        qwen_local:   { target: qwen3.5-9b@local, weight: 2.0 }
      fallback: { mode: parent }
      profile: latency_first

    llm.reason:
      items:
        opus_thinking:  { target: claude-opus-4.7@anthropic, weight: 3.0 }
        gpt5_xhigh:     { target: gpt-5.5@openai, weight: 3.0 }
        grok:           { target: grok-4@xai, weight: 2.0 }
        kimi_thinking:  { target: kimi-k2-thinking@moonshot, weight: 2.0 }
        deepseek_pro:   { target: deepseek-v4-pro@deepseek, weight: 1.5 }
      fallback: { mode: disabled }   # reason 任务不允许降级
      profile: quality_first

    llm.vision:
      items:
        opus:   { target: claude-opus-4.7@anthropic, weight: 3.0 }
        gpt5:   { target: gpt-5.5@openai, weight: 2.5 }
        gemini: { target: gemini-3.1-pro@google, weight: 2.5 }
        qwen_vl:{ target: qwen3.5-vl-32b@local, weight: 1.0 }
      fallback: { mode: parent }

    llm.long:
      items:
        scout:  { target: llama4-scout@local, weight: 3.0 }   # 10M context
        gemini: { target: gemini-3.1-pro@google, weight: 2.0 } # 1M
        sonnet: { target: claude-sonnet-4.6@anthropic, weight: 1.5 }
      fallback: { mode: parent }

    llm.fallback:
      items:
        haiku:      { target: claude-haiku-4.5@anthropic, weight: 1.0 }
        flash_lite: { target: gemini-3.1-flash-lite@google, weight: 1.0 }
        local:      { target: qwen3.5-9b@local, weight: 1.0 }
      fallback: { mode: disabled }

    # ===== Embedding =====
    embedding.text:
      items:
        bge:       { target: bge-m3@local, weight: 3.0 }
        voyage:    { target: voyage-3@voyageai, weight: 2.0 }
        openai:    { target: text-embedding-3-large@openai, weight: 1.0 }
      fallback: { mode: strict }   # 向量空间不通用

    # ===== Image =====
    image.txt2img:
      items:
        flux:     { target: image.flux,     weight: 3.0 }
        seedream: { target: image.seedream, weight: 2.0 }
        imagen:   { target: image.imagen,   weight: 1.5 }
        sd_local: { target: image.sd,       weight: 1.0 }
      fallback: { mode: parent }
      profile: quality_first

    image.img2img:
      items:
        kontext:  { target: image.flux_kontext, weight: 3.0 }
        seedream: { target: image.seedream,     weight: 2.0 }
        gpt_img:  { target: gpt-image-1@openai, weight: 1.5 }
      fallback: { mode: parent }

    image.upscale:
      items:
        topaz: { target: image.topaz, weight: 2.0 }
        esrgan:{ target: image.real_esrgan, weight: 1.0 }
      fallback: { mode: parent }

    # ===== Audio =====
    audio.tts:
      items:
        eleven: { target: eleven-v3@elevenlabs, weight: 3.0 }
        openai: { target: tts-1-hd@openai, weight: 2.0 }
        kokoro: { target: kokoro-82m@local, weight: 1.0 }
      fallback: { mode: strict }   # 音色不能换
      profile: latency_first

    audio.asr:
      items:
        whisper_local: { target: whisper-large-v3@local, weight: 3.0 }
        sensevoice:    { target: sensevoice-small@local, weight: 2.0 }
        whisper_api:   { target: whisper-1@openai, weight: 1.0 }
      fallback: { mode: parent }
      profile: latency_first

    # ===== Video =====
    video.txt2video:
      items:
        kling:    { target: kling-3.0@kling, weight: 2.5 }
        seedance: { target: seedance-2.0@bytedance, weight: 2.5 }
        veo:      { target: veo-3@google, weight: 2.0 }
        sora:     { target: sora-2@openai, weight: 2.0 }
      fallback: { mode: parent }
      profile: quality_first

  global_exact_model_weights:
    # 用户偏好:本地优先(隐私场景)
    qwen3.5-9b@local: 1.5
    bge-m3@local: 1.5

  policy:
    local_only: false
    blocked_provider_instances: []
```

---

## 五、给 v0.4 的待办

1. **多语种 TTS 的目录细分**:`audio.tts` 当前没有按语种区分,但 `eleven-v3` 和 `kokoro-82m` 的语种支持差异巨大。考虑在 `attributes.languages` 上做硬过滤。
2. **图像家族中模型变体的层级**:如 `flux-dev` / `flux-schnell` / `flux-1.1-pro` 都属于 `flux` 家族,但延迟和质量差一档。应在 attributes 中加 `tier: flagship/mid/swift`。
3. **`agent_runtime` 的 fallback 语义**:跨 sandbox 的环境状态不通用(local Docker 启动的容器在云端 E2B 看不到),实际上 strict 更合理。但 strict 会让 fallback 失效,需要在 Provider 层做"会话级 sandbox 粘性"。
4. **多模态 any-to-any 的 schema 收敛**:暂时让 `gpt-4o`、`gemini-3.1-pro`、`qwen-omni` 各自挂多个目录。等业界 API 形态收敛后再考虑合并 namespace。