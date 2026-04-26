# AICC Provider 落地计划

版本：`v0.1-draft`
更新基线：`2026-04-25`
配套文档：

- `doc/aicc/aicc_api设计.md`
- `doc/aicc/aicc 逻辑模型目录.md`
- `doc/aicc/aicc_router.md`

本文用于跟踪 AICC 标准 method 集合的 provider 适配落地状态，目标是让 **`agent` 之外的每个 capability namespace 至少有一个可用 provider**，并把每个 method 的最小落地路径列清楚。`agent.computer_use` 仍处于实验占位阶段，本计划暂不纳入。

---

## 1. 当前实现状态

源码位置：`src/frame/aicc/src/`。

| Provider 适配 | 文件 | 已声明 ApiType |
|---|---|---|
| Anthropic Claude | `claude.rs` | `llm.chat` |
| OpenAI | `openai.rs` | `llm.chat`、`image.txt2img` |
| Google Gemini | `gimini.rs` | `llm.chat`、`image.txt2img` |
| MiniMax | `minimax.rs` | `llm.chat` |
| SN AI Provider | `sn_ai_provider.rs` | （桥接，按宿主声明） |

`model_types.rs::ApiType` 定义了 24 个 method，目前只有 `LlmChat` 和 `ImageTextToImage` 真正接通。

---

## 2. Capability 覆盖缺口

| capability | 已覆盖 method | 缺失 method |
|---|---|---|
| `llm` | `llm.chat` | `llm.completion`（legacy，可不补） |
| `embedding` | — | `embedding.text`、`embedding.multimodal` |
| `rerank` | — | `rerank` |
| `image` | `image.txt2img` | `image.img2img`、`image.inpaint`、`image.upscale`、`image.bg_remove` |
| `vision` | — | `vision.ocr`、`vision.caption`、`vision.detect`、`vision.segment` |
| `audio` | — | `audio.tts`、`audio.asr`、`audio.music`、`audio.enhance` |
| `video` | — | `video.txt2video`、`video.img2video`、`video.video2video`、`video.extend`、`video.upscale` |

---

## 3. 最小落地清单

下表给出**每个 capability 至少跑通一条 route** 所需的最小 provider 集合。优先选开源/本地或可复用现有 cloud key 的 provider，降低接入成本。

### 3.1 Embedding

| 目标 method | 推荐 provider | provider_type | 备注 |
|---|---|---|---|
| `embedding.text` | `bge-m3@local` | local_inference | 开源、多语种；同时可挂 `embedding.multilingual`。 |
| `embedding.text` | `text-embedding-3-large@openai` | cloud_api | 在现有 `openai.rs` 上扩展，复用 API key。 |
| `embedding.multimodal` | `jina-clip-v2@jina` 或 `bge-m3` 多模态变体 | cloud_api / local | 二期补齐。 |

最小集：`bge-m3@local`。如果一期想覆盖多模态可加 `jina-clip-v2@jina`。

### 3.2 Rerank

| 目标 method | 推荐 provider | provider_type |
|---|---|---|
| `rerank` | `bge-reranker-v2-m3@local` | local_inference |
| `rerank` | `cohere-rerank-v3.5@cohere` | cloud_api |

最小集：`bge-reranker-v2-m3@local`。

### 3.3 Image（补齐 txt2img 之外的 method）

| 目标 method | 推荐 provider | provider_type | 备注 |
|---|---|---|---|
| `image.img2img` | `gpt-image-1@openai` | cloud_api | 在 `openai.rs` 上扩。 |
| `image.inpaint` | `gpt-image-1@openai` 或 `flux-fill@bfl` | cloud_api | OpenAI 已有 mask 编辑能力。 |
| `image.upscale` | `real-esrgan@local` | local_inference | 开源、轻量。 |
| `image.bg_remove` | `rmbg-2@bria` 或本地 `u2net` | cloud_api / local | 单一职责 provider。 |

最小集：扩 `openai.rs`（img2img + inpaint）+ `real-esrgan@local` + `rmbg-2`。

### 3.4 Vision

| 目标 method | 推荐 provider | provider_type | 备注 |
|---|---|---|---|
| `vision.ocr` | `paddleocr@local` 或 `florence-2@local` | local_inference | `florence-2` 一家可同时挂 ocr/caption/detect。 |
| `vision.caption` | `florence-2@local` | local_inference | |
| `vision.detect` | `florence-2@local` | local_inference | |
| `vision.segment` | `sam-2@meta`（本地推理） | local_inference | |

最小集：`florence-2@local` + `sam-2@local`。

### 3.5 Audio

| 目标 method | 推荐 provider | provider_type |
|---|---|---|
| `audio.asr` | `whisper-large-v3@local` | local_inference |
| `audio.tts` | `kokoro-82m@local` 或 `eleven-v3@elevenlabs` | local / cloud_api |
| `audio.music` | `musicgen-large@local` 或 `suno-v5@suno` | local / cloud_api |
| `audio.enhance` | `demucs-v4@local` | local_inference |

最小集：`whisper-large-v3@local` + `kokoro-82m@local` + `musicgen-large@local` + `demucs-v4@local`。

### 3.6 Video

| 目标 method | 推荐 provider | provider_type | 备注 |
|---|---|---|---|
| `video.txt2video` | `kling-3.0@kling` 或 `seedance-2.0@bytedance` | cloud_api | 一家可覆盖 txt/img/extend。 |
| `video.img2video` | 同上 | cloud_api | |
| `video.video2video` | `kling-3.0-omni@kling` | cloud_api | |
| `video.extend` | `kling-3.0@kling` | cloud_api | 注意 §11.5 fallback 约束，需保留 `continuation_handle`。 |
| `video.upscale` | `topaz-video-upscale@topaz` 或 `real-esrgan` 视频版 | cloud_api / local | |

最小集：`kling-3.0@kling`（覆盖 4 个 method）+ `topaz-video-upscale`。

---

## 4. 汇总：最少新增 provider 数

让 7 个非 agent capability 全部 "至少 1 条 route 可用"，最小新增适配器：

| 编号 | Provider | 覆盖的 method |
|---|---|---|
| 1 | `bge-m3@local` | `embedding.text`（含多语种） |
| 2 | `bge-reranker-v2-m3@local` | `rerank` |
| 3 | OpenAI 适配扩 `gpt-image-1` | `image.img2img`、`image.inpaint` |
| 4 | `real-esrgan@local` | `image.upscale` |
| 5 | `rmbg-2@bria` | `image.bg_remove` |
| 6 | `florence-2@local` | `vision.ocr`、`vision.caption`、`vision.detect` |
| 7 | `sam-2@local` | `vision.segment` |
| 8 | `whisper-large-v3@local` | `audio.asr` |
| 9 | `kokoro-82m@local` | `audio.tts` |
| 10 | `musicgen-large@local` | `audio.music` |
| 11 | `demucs-v4@local` | `audio.enhance` |
| 12 | `kling-3.0@kling` | `video.txt2video`、`video.img2video`、`video.video2video`、`video.extend` |
| 13 | `topaz-video-upscale@topaz` | `video.upscale` |

合计 **13 个 provider**（其中第 3 项是在现有 `openai.rs` 上扩 method，不新增文件）。

---

## 5. 落地顺序建议

按"业务价值 / 接入复杂度"排序，分四批推进。每批完成后跑通 DV test 再开下一批。

### Batch 1：文本类闭环（embedding + rerank）

- `bge-m3@local`、`bge-reranker-v2-m3@local`
- 同时把 `payload.input_json` / artifact 大小阈值（`prefer_artifact=auto`）跑一遍。
- 退出条件：知识库类调用方可以同时跑 embed + rerank。

### Batch 2：Image 补齐 + Vision OCR

- 在 `openai.rs` 扩 `image.img2img` / `image.inpaint`。
- 新增 `real-esrgan@local`、`rmbg-2`、`florence-2@local`。
- 退出条件：图片处理类工作流（生成 → 编辑 → 超分 → 抠图 → OCR）端到端可用。

### Batch 3：Audio 全套

- `whisper-large-v3@local`、`kokoro-82m@local`、`demucs-v4@local`、`musicgen-large@local`。
- 退出条件：会议转录、TTS 朗读、音轨降噪、配乐生成可独立跑通；`audio.asr` 多 `output_formats` 的 artifact 命名按 §10.2 规范。

### Batch 4：Video + Vision Segment

- `kling-3.0@kling`、`topaz-video-upscale`、`sam-2@local`。
- 退出条件：Video method 全部走 task-manager 异步路径，`continuation_handle` 在 `extend` 上验证通过。

---

## 6. 每接入一个 provider 的强制清单

参考 `aicc_api设计.md §16` M3 阶段要求，每个 provider 落地前后必须同步：

1. `ProviderInventory.models[].methods` 声明硬过滤 method。
2. `exact_model = <provider_model_id>@<provider_instance_name>` 命名校验。
3. `attributes.privacy` 使用 `local` / `private_cloud` / `public_cloud` / `public_cloud_no_log` 枚举。
4. 在 Provider Adapter 内做 method schema → provider-native 的双向映射，禁止往 `payload` 新增并行通道。
5. 资源输入只接受 `ResourceRef`，且只在最后一跳读取 bytes。
6. 注册 fallback 规则：embedding 必须 strict 同 `embedding_space_id`；audio.tts `speaker_similarity_required=true` 时禁止跨 provider；video 一旦 Started 不再跨 provider 重试。
7. 接入 task-manager 的 `Pending` / `Running` / `Completed` / `Failed` 状态写入。
8. 单元测试或 DV test 至少覆盖：成功路径、provider 错误、取消、idempotency 命中。

---

## 7. 暂不纳入

- `agent.computer_use`：仍处于占位阶段，依赖外部 Agent Runtime/沙箱状态，等 Agent Runtime 牵头时再补。
- `llm.completion`：legacy，新 provider 不必声明。
- `multimodal` any-to-any：等业界 API 形态收敛后再考虑。
