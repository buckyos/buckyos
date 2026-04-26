# AICC 模型目录树设计 

本文档是 `aicc_router.md` 的配套设计,定义 AICC 标准化的:

1. **API Type 枚举**:Provider 必须从这个有限集合声明能力,新增需主版本升级。
2. **一级逻辑目录**:用户/Agent 调用 AICC 时使用的 namespace。
3. **Provider 逻辑挂点 + 物理型号索引**:供 `logical_mounts` 和 `items` 软链接使用。

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

## 三、模型逻辑挂点与物理型号索引

### 设计约定

- **逻辑挂点**是 Provider 内稳定的 tier 名,永远代表该 tier 的最新可用版本,如 `llm.opus`、`llm.gpt`、`llm.gemini-flash`。
- **物理型号**是带版本号的具体快照,如 `claude-opus-4.7@anthropic`、`gpt-5.5@openai`,只用于复现、锁定、审计和 Provider 侧实际路由。
- **任务抽象目录**与 Provider 逻辑挂点正交。`llm.plan`、`llm.swift`、`llm.reason` 这类抽象分组只链接到 Provider 逻辑挂点,不直接链接物理型号。
- **家族目录**保留为 UI 展示、用户偏好和物理型号索引用途。物理型号可以同时归入家族目录,但不作为任务抽象目录的直接 target。

### 3.1 `llm` Provider 逻辑挂点目录

```
llm
├── gpt-pro              # OpenAI 研究级深度推理
├── gpt                  # OpenAI 旗舰主力
├── gpt-mini             # OpenAI 低延迟/低成本主力
├── gpt-nano             # OpenAI 极轻量/批量场景
├── opus                 # Anthropic 旗舰推理与 agentic 编排
├── sonnet               # Anthropic 性价比主力
├── haiku                # Anthropic 轻量快速
├── gemini-deepthink     # Google Deep Think 极限推理
├── gemini-pro           # Google Gemini Pro 旗舰推理
├── gemini-flash         # Google Gemini Flash 平衡速度/能力
├── gemini-flash-lite    # Google Gemini Flash-Lite 高吞吐极轻量
├── grok-heavy           # xAI Grok Heavy 多 agent 重型
├── grok                 # xAI Grok 旗舰
├── grok-fast            # xAI Grok Fast 低成本低延迟
├── deepseek-pro         # DeepSeek 旗舰
├── deepseek-flash       # DeepSeek 平衡速度与成本
├── deepseek-reasoner    # DeepSeek R 系列独立推理线
├── qwen-max             # Alibaba Qwen 旗舰
├── qwen-plus            # Alibaba Qwen 主力均衡
├── qwen-coder           # Alibaba Qwen 编码专精
├── qwen-small           # Alibaba Qwen 本地/边缘部署
├── glm                  # Z.ai GLM 旗舰
├── glm-flash            # Z.ai GLM 轻量
├── kimi                 # Moonshot Kimi 旗舰
└── kimi-thinking        # Moonshot Kimi 推理线
```

#### 主流 AI Provider 逻辑挂点清单(2026-04 基线)

| Provider | 逻辑挂点 | 当前指向 | 定位 |
|---|---|---|---|
| OpenAI | `llm.gpt-pro` | GPT-5.5 Pro | 研究级深度推理 |
| OpenAI | `llm.gpt` | GPT-5.5 | 旗舰,复杂推理与编码 |
| OpenAI | `llm.gpt-mini` | GPT-5.4-mini | 低延迟、低成本主力 |
| OpenAI | `llm.gpt-nano` | GPT-5.4-nano | 极轻量、批量场景 |
| Anthropic | `llm.opus` | Claude Opus 4.7 | 最强推理与 agentic 编排 |
| Anthropic | `llm.sonnet` | Claude Sonnet 4.6 | 性价比主力,接近 Opus |
| Anthropic | `llm.haiku` | Claude Haiku 4.5 | 轻量快速 |
| Google | `llm.gemini-deepthink` | Gemini 3 Deep Think | Ultra 订阅专享,极限推理 |
| Google | `llm.gemini-pro` | Gemini 3.1 Pro | 旗舰推理 |
| Google | `llm.gemini-flash` | Gemini 3 Flash | 平衡速度/能力 |
| Google | `llm.gemini-flash-lite` | Gemini 3.1 Flash-Lite | 高吞吐成本敏感极轻量 |
| xAI | `llm.grok-heavy` | Grok 4.3 / Grok 4 Heavy | 多 agent 并行重型 |
| xAI | `llm.grok` | Grok 4.20 | 旗舰,四 agent 架构 |
| xAI | `llm.grok-fast` | Grok 4.1 Fast | 低成本低延迟 |
| DeepSeek | `llm.deepseek-pro` | DeepSeek-V4-Pro | 旗舰 |
| DeepSeek | `llm.deepseek-flash` | DeepSeek-V4-Flash | 平衡速度与成本 |
| DeepSeek | `llm.deepseek-reasoner` | DeepSeek R 系列 | 独立推理线 |
| Alibaba | `llm.qwen-max` | Qwen 3.6 Plus | 旗舰,1M context、原生 function calling |
| Alibaba | `llm.qwen-plus` | Qwen 3.5 系列 | 主力均衡 |
| Alibaba | `llm.qwen-coder` | Qwen 3 Coder | 编码专精线 |
| Alibaba | `llm.qwen-small` | Qwen 3.5 9B / 27B | 本地/边缘部署 |
| Z.ai | `llm.glm` | GLM-5.1 | 旗舰 |
| Z.ai | `llm.glm-flash` | GLM-4.7 Flash / GLM-5 Flash | 轻量本地 |
| Moonshot | `llm.kimi` | Kimi K2.6 | 旗舰 |
| Moonshot | `llm.kimi-thinking` | Kimi K2 Thinking | 推理线 |

#### 物理型号索引示例

物理型号只作为 Provider 逻辑挂点的当前指向或可锁定快照存在。`llm.plan`、`llm.swift` 等任务抽象目录不直接引用下表条目。

| 逻辑挂点 | 物理型号 | 家族 | api_type | attributes.tier |
|---|---|---|---|---|
| `llm.gpt-pro` | `gpt-5.5-pro@openai` | openai | llm.chat | flagship |
| `llm.gpt` | `gpt-5.5@openai` | openai | llm.chat | flagship |
| `llm.gpt-mini` | `gpt-5.4-mini@openai` | openai | llm.chat | mid |
| `llm.gpt-nano` | `gpt-5.4-nano@openai` | openai | llm.chat | nano |
| `llm.opus` | `claude-opus-4.7@anthropic` | claude | llm.chat | flagship |
| `llm.sonnet` | `claude-sonnet-4.6@anthropic` | claude | llm.chat | mid |
| `llm.haiku` | `claude-haiku-4.5@anthropic` | claude | llm.chat | nano |
| `llm.gemini-deepthink` | `gemini-3-deepthink@google` | gemini | llm.chat | flagship |
| `llm.gemini-pro` | `gemini-3.1-pro@google` | gemini | llm.chat | flagship |
| `llm.gemini-flash` | `gemini-3-flash@google` | gemini | llm.chat | mid |
| `llm.gemini-flash-lite` | `gemini-3.1-flash-lite@google` | gemini | llm.chat | nano |
| `llm.grok-heavy` | `grok-4-heavy@xai` | grok | llm.chat | flagship |
| `llm.grok` | `grok-4.20@xai` | grok | llm.chat | flagship |
| `llm.grok-fast` | `grok-4.1-fast@xai` | grok | llm.chat | mid |
| `llm.deepseek-pro` | `deepseek-v4-pro@deepseek` | deepseek | llm.chat | flagship |
| `llm.deepseek-flash` | `deepseek-v4-flash@deepseek` | deepseek | llm.chat | mid |
| `llm.deepseek-reasoner` | `deepseek-r@deepseek` | deepseek | llm.chat | flagship |
| `llm.qwen-max` | `qwen-3.6-plus@alibaba` | qwen | llm.chat | flagship |
| `llm.qwen-plus` | `qwen-3.5-plus@alibaba` | qwen | llm.chat | mid |
| `llm.qwen-coder` | `qwen-3-coder@alibaba` | qwen | llm.chat | flagship |
| `llm.qwen-small` | `qwen-3.5-9b@local` | qwen | llm.chat | nano |
| `llm.kimi` | `kimi-k2.6@moonshot` | kimi | llm.chat | flagship |
| `llm.kimi-thinking` | `kimi-k2-thinking@moonshot` | kimi | llm.chat | flagship |
| `llm.glm` | `glm-5.1@zai` | glm | llm.chat | flagship |
| `llm.glm-flash` | `glm-5-flash@zai` | glm | llm.chat | nano |

**Reasoning 角色专属**:`llm.reason` 默认只挂载支持强 reasoning / thinking 的逻辑挂点,如 `llm.gemini-deepthink`、`llm.opus`、`llm.gpt-pro`、`llm.grok-heavy`、`llm.deepseek-reasoner`、`llm.kimi-thinking`。具体物理型号由对应 Provider 挂点解析。

**Vision 角色**:`llm.vision` 挂载支持图像输入的逻辑挂点,如 `llm.gemini-pro`、`llm.opus`、`llm.gpt`。纯文本逻辑挂点通过 Provider metadata 过滤。

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
        opus:     { target: llm.opus,      weight: 2.5 }
        gpt_pro:  { target: llm.gpt-pro,   weight: 2.5 }
        gemini:   { target: llm.gemini-pro, weight: 2.4 }
        qwen_max: { target: llm.qwen-max,  weight: 1.8 }
        deepseek: { target: llm.deepseek-pro, weight: 1.5 }
      fallback: { mode: parent }
      profile: quality_first

    llm.code:
      items:
        opus:        { target: llm.opus,        weight: 2.5 }
        gpt_pro:     { target: llm.gpt-pro,     weight: 2.5 }
        gemini:      { target: llm.gemini-pro,  weight: 2.4 }
        qwen_coder:  { target: llm.qwen-coder,  weight: 2.0 }
        kimi:        { target: llm.kimi,        weight: 2.0 }
        glm:         { target: llm.glm,         weight: 1.5 }
        deepseek:    { target: llm.deepseek-pro, weight: 1.5 }
      fallback: { mode: parent }

    llm.swift:
      items:
        haiku:        { target: llm.haiku,             weight: 2.5 }
        flash_lite:   { target: llm.gemini-flash-lite, weight: 2.5 }
        gpt_nano:     { target: llm.gpt-nano,          weight: 2.5 }
        grok_fast:    { target: llm.grok-fast,         weight: 2.0 }
        qwen_small:   { target: llm.qwen-small,        weight: 2.0 }
        glm_flash:    { target: llm.glm-flash,         weight: 1.5 }
      fallback: { mode: parent }
      profile: latency_first

    llm.reason:
      items:
        gemini_deepthink: { target: llm.gemini-deepthink,  weight: 2.5 }
        opus:             { target: llm.opus,              weight: 2.5 }
        gpt_pro:          { target: llm.gpt-pro,           weight: 2.5 }
        grok_heavy:       { target: llm.grok-heavy,        weight: 2.0 }
        kimi_thinking:    { target: llm.kimi-thinking,     weight: 2.0 }
        deepseek_reasoner: { target: llm.deepseek-reasoner, weight: 2.0 }
      fallback: { mode: disabled }   # reason 任务不允许降级
      profile: quality_first

    llm.vision:
      items:
        opus:   { target: llm.opus,       weight: 2.5 }
        gpt:    { target: llm.gpt,        weight: 2.5 }
        gemini: { target: llm.gemini-pro, weight: 2.5 }
        qwen:   { target: llm.qwen-max,   weight: 1.0 }
      fallback: { mode: parent }

    llm.long:
      items:
        gemini:  { target: llm.gemini-pro, weight: 2.0 }
        qwen:    { target: llm.qwen-max,   weight: 2.0 }
        sonnet:  { target: llm.sonnet,     weight: 1.5 }
      fallback: { mode: parent }

    llm.fallback:
      items:
        haiku:      { target: llm.haiku,             weight: 1.0 }
        flash_lite: { target: llm.gemini-flash-lite, weight: 1.0 }
        gpt_nano:   { target: llm.gpt-nano,          weight: 1.0 }
        qwen_small: { target: llm.qwen-small,        weight: 1.0 }
      fallback: { mode: disabled }

    # ===== Embedding =====
    embedding.text:
      items:
        bge:       { target: bge-m3@local, weight: 2.0 }
        voyage:    { target: voyage-3@voyageai, weight: 2.0 }
        openai:    { target: text-embedding-3-large@openai, weight: 1.0 }
      fallback: { mode: strict }   # 向量空间不通用

    # ===== Image =====
    image.txt2img:
      items:
        flux:     { target: image.flux,     weight: 2.5 }
        seedream: { target: image.seedream, weight: 2.0 }
        imagen:   { target: image.imagen,   weight: 2.5 }
        sd_local: { target: image.sd,       weight: 1.0 }
      fallback: { mode: parent }
      profile: quality_first

    image.img2img:
      items:
        kontext:  { target: image.flux_kontext, weight: 2.0 }
        seedream: { target: image.seedream,     weight: 2.0 }
        gpt_img:  { target: gpt-image-1@openai, weight: 2.0 }
      fallback: { mode: parent }

    image.upscale:
      items:
        topaz: { target: image.topaz, weight: 2.0 }
        esrgan:{ target: image.real_esrgan, weight: 1.0 }
      fallback: { mode: parent }

    # ===== Audio =====
    audio.tts:
      items:
        eleven: { target: eleven-v3@elevenlabs, weight: 2.5 }
        openai: { target: tts-1-hd@openai, weight: 2.0 }
        kokoro: { target: kokoro-82m@local, weight: 1.0 }
      fallback: { mode: strict }   # 音色不能换
      profile: latency_first

    audio.asr:
      items:
        whisper_local: { target: whisper-large-v3@local, weight: 2.5 }
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


  policy:
    local_only: false
    blocked_provider_instances: []
```

---

## 五、给 v0.4 的待办

1. **多语种 TTS 的目录细分**:`audio.tts` 当前没有按语种区分,但 `eleven-v3` 和 `kokoro-82m` 的语种支持差异巨大。考虑在 `attributes.languages` 上做硬过滤。
2. **图像家族中模型变体的层级**:如 `flux-dev` / `flux-schnell` / `flux-1.1-pro` 都属于 `flux` 家族,但延迟和质量差一档。应在 attributes 中加 `tier: flagship/mid/swift`。
3. **`agent_runtime` 的 fallback 语义**:跨 sandbox 的环境状态不通用(local Docker 启动的容器在云端 E2B 看不到),实际上 strict 更合理。但 strict 会让 fallback 失效,需要在 Provider 层做"会话级 sandbox 粘性"。
4. **多模态 any-to-any 的 schema 收敛**:暂时让 `llm.gpt`、`llm.gemini-pro`、`llm.qwen-max` 这类逻辑挂点各自挂多个目录。等业界 API 形态收敛后再考虑合并 namespace。
