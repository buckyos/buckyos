# AICC 模型目录树设计 

本文档是 `aicc_router.md` 的配套设计,定义 AICC 标准化的:

1. **API Type 枚举**:Provider 必须从这个有限集合声明能力,新增需主版本升级。
2. **一级逻辑目录**:用户/Agent 调用 AICC 时使用的 namespace。
3. **厂商规格、模型家族与物理 instance**:LLM 通过 `items` 引用规格和固定家族预设，非 LLM 保留原有挂点设计。

LLM 实现更新：2026-09-25，依据 `service/model_defaults.rs` 头部契约；非 LLM 内容保留 2026-04-24 基线。以下 LLM 结构已实现，离线验收见 [实现报告](model_driver_v2_implementation.md)，字段与 OpenAI 示例见 [Metadata 目标契约](driver_metadata_schema.md#llm-target-contract-vendor-specifications-and-model-families)。

---

## 一、API Type 枚举(必要项)

API Type 决定 request/response schema。**Provider 不能自定义 api_type**,必须从下表选择;同一精确模型可声明多个 api_type。

### 1.1 文本类

| api_type | 输入 | 输出 | 说明 |
|---|---|---|---|
| `llm` | `messages[]`(可含 image/audio block) | `message` + 可选 `tool_calls` | 主流 chat completion,事实标准 |
| `embedding.text` | `string \| string[]` | `number[][]` | 文本/代码 embedding |
| `embedding.multimodal` | `text \| image \| (text+image)` | `number[][]` | CLIP 类跨模态 embedding |
| `decision` | `state, questions[]` | `answers[]`（保留概率） | 独立混合类型问题求值 |
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
| `audio.tts` | `text + voice contract` | `audio` | Provider/模型无关的文转语音 |
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
| `llm` | `llm` | namespace-only；无 Parent fallback | 功能目录分别配置 |
| `embedding` | `embedding.text`、`embedding.multimodal` | **strict**(向量空间不通用) | latency_first |
| `decision` | `decision` | strict | cost_first |
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
├── vision     # 需要传图的对话(VLM)
├── swift      # 极速响应(短回复、低延迟)
├── summarize  # 总结/抽取，通过规格选择
├── translate  # 翻译
└── fallback   # 默认空，只接收显式配置
```

支持的 api_type：`llm`。纯文本 completion 由调用方转换为单条 message，不定义独立 api_type。

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

### LLM 设计约定（目标）

- **功能目录**如 `llm.plan`、`llm.chat`，通过带偏好权重的 item 引用厂商规格。功能目录不直接接收 inventory 的 exact model，也不使用通用 Auto/Hybrid 按能力自动吸入模型。
- **规格目录**由每个原厂 Model Driver 的 `specs` 独立声明，路径为 `llm.{spec.id}`。规格名不带版本号；既可以是产品线，也可以是 code/highspeed 等专用规格，没有跨厂商统一档位。
- **模型家族目录**通常为 `llm.{归一化官方模型ID}`，如 `llm.gpt-5-6-sol`；它是确定官方模型的可选择入口，也是多个物理 instance 的汇集处。官方 ID 中的小数点等分隔符归一为连字符，保留原始 ID 用于匹配/调用，并校验与功能、规格和其他家族的重名。
- **固定思考预设**表示为 `llm.gpt-5-6-sol:high`；`:high` 是预设选择器，不是逻辑子目录。模型条目明确自己以哪个预设归入唯一规格，家族直选的默认预设另行声明。只能声明实际支持的强度；`none`、`thinking`、`native` 分别表示关闭、仅开关的开启、不可调原生行为。
- **物理 instance**沿用 exact model 身份，例如 `gpt-5.6-sol:reasoning-high@provider-a`。不同渠道的模型名先由 Provider Rules/discovery 归一到同一官方身份，才能共享家族。渠道不支持所需预设时跳过该实例，不能静默换成另一个思考强度。
- **能力约束与偏好分开**：家族、预设和实例仍必须通过原任务/request 的能力、库存、Provider 状态及策略筛选；能力合格不等于可以绕过规格归属。

### 3.1 `llm` 规格、家族与实例

以下是 OpenAI 目标结构的局部示意。五个通用规格为 `gpt-nano/mini/standard/pro/max`，`gpt-codex` 是额外的专用规格。具体归档及功能权重以 `model_defaults.rs` 头部设计为准，规格由 metadata 声明，功能权重由 builtin overlay 维护。

```text
llm
├── plan
│   └── gpt_pro -> llm.gpt-pro (2.3，功能偏好)
├── gpt-nano                  [规格；可为空]
├── gpt-mini                  [规格；可为空]
├── gpt-standard              [规格；可为空]
├── gpt-pro                   [规格]
│   ├── llm.gpt-5-6-sol -> llm.gpt-5-6-sol:high (56，metadata llm.weight)
│   └── llm.gpt-5-5-pro -> llm.gpt-5-5-pro:high (55，metadata llm.weight)
├── gpt-max                   [规格；可为空]
├── gpt-codex                 [专用规格；可为空]
├── gpt-5-6-sol               [动态家族]
│   └── :high                [固定预设；不是 .high 子目录]
│       ├── provider_a -> gpt-5.6-sol:reasoning-high@provider-a
│       └── provider_b -> gpt-5.6-sol:reasoning-high@provider-b
└── gpt-5-5-pro               [动态家族]
    └── :high
        └── provider_a -> gpt-5.5-pro:reasoning-high@provider-a
```

上图假设两个 Provider 的调用 ID 相同；渠道 ID 不同时，exact target 使用各自真实 ID，家族仍按官方身份合并。`56/55` 是 model-driver metadata 为各模型声明的 `llm.weight`，Registry 原样写入规格到家族的默认 item 权重，路由不从模型名解析版本；该权重只在同一规格内比较，不与功能偏好 `2.3` 相乘，也不与其它规格或厂商的权重比较，因此不同系列重复使用 `55` 没有冲突。多个 Provider 不增加规格到家族的引用次数或权重；家族到 instance 的默认权重为 `1.0`。两层权重都可通过 `item_overrides` 覆盖（只改权重，不改成员归属），覆盖值、默认值及来源在目录视图和 trace 中可见。

零 Provider 时只保留功能目录、声明的规格及功能到规格的引用。metadata 与有效 inventory 相交后才创建家族及预设；最后一个对应实例消失时删除动态家族和引用，规格恢复为空。官方 Provider 下线不删除其他渠道仍可提供的家族，也不删除其模型定义。

选择分两阶段。阶段一逐层展开：功能目录、规格、家族预设各自只展开本层可用 item 中权重最大的一组，并列全部展开；某组没有任何可用候选（无库存、admission 不满足、运行状态或策略被硬过滤）时才尝试同层下一权重组，全部不可用则该目录为空。同权重规格不按规格 ID 提前淘汰，而是各自展开后共同进入阶段二。阶段二把展开得到的合格 instance 合并成候选池，按调度 profile（默认成本优先）选择，策略项全部相同时按默认顺序（配置书写顺序；家族按路径、instance 按 exact model 名）。`stability = experimental` 只是准入条件：请求未允许实验版时硬过滤，允许后不再被隐式排在稳定版之后。运行时 failover 只在本轮候选池内进行。

任务、规格、家族没有隐式 Parent fallback；`llm` 不收集模型，`llm.fallback` 默认空。家族直选默认 strict，使用声明的默认预设，不自动升级其他家族。显式 fallback 保留原请求及原任务约束。每个规格须被功能引用或标记 `direct_only`，后者不能通过功能或 fallback 暗中接入。

以下非 LLM 家族目录保留原有设计，不套用本节的规格规则。

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

## 四、全局 routing config 示意

以下沿用本文 YAML 展示形式说明功能到规格的偏好，不是当前公共 DTO 的可直接写入配置；LLM 部分为新设计，非 LLM 示例保留原基线。实际 overlay 结构与 factory/system/user/session 顺序见 [冻结设计](frozen_model_driver_and_logical_model_fs.md)。规格及动态家族由 metadata/inventory 生成，无须在路由偏好里重复声明。

```yaml
routing_config:
  schema_version: 1
  revision: 0
  default_profile: balanced
  
  logical_tree:
    # ===== LLM：局部示意，其余功能权重见 model_defaults.rs 头部契约 =====
    llm.chat:
      items:
        - { name: gpt_standard, target: llm.gpt-standard, weight: 2.2 }
        - { name: gpt_mini, target: llm.gpt-mini, weight: 1.4 }
      fallback: { mode: disabled }

    llm.plan:
      items:
        - { name: gpt_pro, target: llm.gpt-pro, weight: 2.3 }
        - { name: gpt_max, target: llm.gpt-max, weight: 2.6 }
      fallback: { mode: disabled }
      profile: quality_first

    llm.code:
      items:
        - { name: gpt_codex, target: llm.gpt-codex, weight: 2.2 }
        - { name: gpt_standard, target: llm.gpt-standard, weight: 2.1 }
      fallback: { mode: disabled }

    llm.swift:
      items:
        - { name: gpt_nano, target: llm.gpt-nano, weight: 1.1 }
      fallback: { mode: disabled }
      profile: latency_first

    llm.fallback:
      items: []
      fallback: { mode: disabled }

    # ===== Embedding =====
    embedding.text:
      items:
        - { name: bge, target: bge-m3@local, weight: 2.0 }
        - { name: voyage, target: voyage-3@voyageai, weight: 2.0 }
        - { name: openai, target: text-embedding-3-large@openai, weight: 1.0 }
      fallback: { mode: strict }   # 向量空间不通用

    # ===== Image =====
    image.txt2img:
      items:
        - { name: seedream, target: image.seedream, weight: 2.0 }
      fallback: { mode: parent }
      profile: quality_first

    image.img2img:
      items:
        - { name: kontext, target: image.flux_kontext, weight: 2.0 }
        - { name: gpt_img, target: gpt-image-1@openai, weight: 2.0 }
      fallback: { mode: parent }

    image.upscale:
      items:
        - { name: topaz, target: image.topaz, weight: 2.0 }
        - { name: esrgan, target: image.real_esrgan, weight: 1.0 }
      fallback: { mode: parent }

    # ===== Audio =====
    audio.tts:
      items:
        - { name: eleven, target: eleven-v3@elevenlabs, weight: 2.5 }
        - { name: openai, target: tts-1-hd@openai, weight: 2.0 }
        - { name: kokoro, target: kokoro-82m@local, weight: 1.0 }
      fallback: { mode: strict }   # 音色不能换
      profile: latency_first

    audio.asr:
      items:
        - { name: whisper_local, target: whisper-large-v3@local, weight: 2.5 }
        - { name: sensevoice, target: sensevoice-small@local, weight: 2.0 }
        - { name: whisper_api, target: whisper-1@openai, weight: 1.0 }
      fallback: { mode: parent }
      profile: latency_first

    # ===== Video =====
    video.txt2video:
      items:
        - { name: kling, target: kling-3.0@kling, weight: 2.5 }
        - { name: seedance, target: seedance-2.0@bytedance, weight: 2.5 }
        - { name: veo, target: veo-3@google, weight: 2.0 }
        - { name: sora, target: sora-2@openai, weight: 2.0 }
      fallback: { mode: parent }
      profile: quality_first


  policy:
    local_only: false
    blocked_provider_instances: []
```

LLM 目标中，功能 item 只引用已声明规格；Provider inventory 的物理实例归入动态家族预设，不直接挂入功能目录。显式 overlay 调整偏好后仍须通过规格引用和图校验。

---

## 五、给 v0.4 的待办

1. **多语种 TTS 的目录细分**:`audio.tts` 当前没有按语种区分,但 `eleven-v3` 和 `kokoro-82m` 的语种支持差异巨大。考虑在 `attributes.languages` 上做硬过滤。
2. **图像家族中模型变体的层级**:如 `flux-dev` / `flux-schnell` / `flux-1.1-pro` 都属于 `flux` 家族,但延迟和质量差一档。应在 attributes 中加 `tier: flagship/mid/swift`。
3. **`agent_runtime` 的 fallback 语义**:跨 sandbox 的环境状态不通用(local Docker 启动的容器在云端 E2B 看不到),实际上 strict 更合理。但 strict 会让 fallback 失效,需要在 Provider 层做"会话级 sandbox 粘性"。
4. **多模态 any-to-any 的 schema 收敛**:暂时让 OpenAI GPT 角色挂点、`llm.gemini-pro`、`llm.qwen-max` 这类逻辑挂点各自挂多个目录。等业界 API 形态收敛后再考虑合并 namespace。

## Decision 逻辑入口（2026-09-25）

一级入口新增 `decision`，采用非 LLM Hybrid 挂点、strict 回退和 cost_first 调度。零 Provider 时保留目录但无候选。TypeSafe 模型挂点为 `decision`、`decision.typesafe` 与模型子路径，不归入 LLM 规格或 effort。题型、结构化输入与容量约束在逻辑、exact 和 preview 路由均生效。完整契约、示例及核验来源见 [Decision API](decision_api.md)。
