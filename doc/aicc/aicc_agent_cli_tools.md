# AICC Agent CLI Tools

版本：`v0.1-draft`
更新基线：`2026-05-12`

本文定义一组面向 Agent 的 AI 能力 CLI。CLI 是 AICC kRPC API 的薄封装，用于让 Agent 在 shell / workflow / task runner 中直接调用典型 AI 能力，例如：

```bash
gen_image "A precise product photo of a matte black desk lamp." result.png
```

本文不包含原始 LLM 推理类命令。`llm.chat`、`llm.completion` 仍由 Agent Runtime / SDK 直接使用，不包装成通用 shell 工具。

相关协议：

- `src/kernel/buckyos-api/src/aicc_client.rs`
- `doc/aicc/aicc_api设计.md`
- `doc/aicc/krpc_aicc_calling_guide.md`

---

## 1. 目标

Agent CLI 的目标是把常用 AI 能力变成可组合、可脚本化、可观察的本地命令：

1. 文生图、图生图、局部重绘、超分、抠图。
2. OCR、图片描述、检测、分割。
3. TTS、ASR、音乐生成、音频增强。
4. 文生视频、图生视频、视频编辑、续写、超分。
5. provider / quota 等服务辅助查询命令。

CLI 只列出最终结果对用户或交付物有直接意义的能力。`embedding.text`、`rerank` 这类中间计算能力应保留在 SDK / Agent Runtime 内部使用，不作为独立面向 Agent 的短命令暴露。

CLI 不重新定义 AI 协议。每个命令只负责：

1. 把命令行参数转换成 `AiMethodRequest.payload.input_json`。
2. 把本地输入文件转换成 `ResourceRef`。
3. 调用 `/kapi/aicc` 对应 method。
4. 把 `AiResponseSummary.artifacts` 或 `extra` 落到本地文件 / stdout。

---

## 2. 总体约定

### 2.1 命令命名

命令使用短动词 + 能力对象，便于 Agent 直接写 shell：

```bash
gen_image "prompt" result.png
ocr_image page.png result.txt
text_to_speech "text" result.mp3
speech_to_text meeting.mp3 transcript.txt
gen_video "prompt" result.mp4
```

不要求用户记住 AICC method 名。命令内部固定映射到标准 method。命令名避免使用三字母缩写，降低和系统命令、shell alias、包管理器工具冲突的概率。

### 2.2 通用参数

所有命令支持以下通用参数：

```text
--model <alias>              逻辑模型名或精确模型名，默认由命令决定
--profile <balanced|cheap|fast|quality>
--max-cost <usd>
--max-latency-ms <ms>
--no-fallback
--idempotency-key <key>
--trace-id <trace_id>
--json                       输出完整 AiMethodResponse JSON 到 stdout
--timeout <seconds>          当前 CLI 进程最长等待时间
```

`--profile` 映射到 `RoutePolicy.profile`：

| CLI value | AICC value | 语义 |
|---|---|---|
| `cheap` | `cheap` | 成本优先 |
| `fast` | `fast` | 延迟优先 |
| `balanced` | `balanced` | 综合排序 |
| `quality` | `quality` | 质量优先 |

`--no-fallback` 等价于：

```json
{
  "policy": {
    "allow_fallback": false,
    "runtime_failover": false
  }
}
```

### 2.3 输入资源

本地输入文件优先写成 BuckyOS `FileObject`，再以 `ResourceRef::NamedObject` 传给 AICC：

```json
{
  "kind": "named_object",
  "obj_id": "chunk:source-image"
}
```

实现允许在小文件或调试模式下使用 `ResourceRef::Base64`，但 CLI 对外不暴露协议差异。

URL 输入通过 `--url` 或参数值的 URL scheme 识别，转换为：

```json
{
  "kind": "url",
  "url": "https://example.com/image.png",
  "mime_hint": "image/png"
}
```

### 2.4 输出

默认行为：

1. 单文件输出命令把第一个匹配 artifact 下载到指定路径。
2. 多文件输出命令要求输出目录。
3. 结构化结果写 JSON 文件；如果未指定输出文件，则写 stdout。
4. `--json` 时不做精简输出，直接输出完整 `AiMethodResponse`。

输出文件不存在时创建，存在时覆盖。CLI 不做交互确认，因为这些工具面向 Agent 自动化。

### 2.5 退出码

| code | 含义 |
|---|---|
| `0` | 成功 |
| `1` | 参数错误 |
| `2` | AICC 调用失败 |
| `3` | Provider / route 失败 |
| `4` | CLI 进程等待超时或被外部结束 |
| `5` | 输入 / 输出文件处理失败 |

错误信息写 stderr。`--json` 下错误也应输出结构化 JSON，便于 Agent 解析。

---

## 3. Image CLI

### 3.1 `gen_image`

文生图。映射到 `image.txt2img`。

```bash
gen_image "A precise product photo of a matte black desk lamp." result.png
```

参数：

```text
gen_image <prompt> <output_image>
  --negative <text>
  --n <count>
  --aspect <1:1|16:9|9:16|4:3|3:4>
  --size <1024x1024|...>
  --quality <low|medium|high>
  --seed <int>
  --format <png|jpg|webp>
```

AICC mapping：

```text
method: image.txt2img
capability: image
model.alias default: image.txt2img
payload.input_json:
  prompt
  negative_prompt
  n
  aspect_ratio
  quality
  seed
  output.media_type
  output.size
```

示例：

```bash
gen_image "logo mark for a distributed personal cloud OS" logo.png \
  --aspect 1:1 \
  --quality high \
  --profile quality
```

### 3.2 `edit_image`

图生图。映射到 `image.img2img`。

```bash
edit_image source.png "Change the background to a sunny beach." result.png
```

参数：

```text
edit_image <input_image> <prompt> <output_image>
  --strength <0.0-1.0>
  --format <png|jpg|webp>
```

### 3.3 `inpaint_image`

局部重绘。映射到 `image.inpaint`。

```bash
inpaint_image image.png mask.png "Add a vase of flowers on the table." result.png
```

参数：

```text
inpaint_image <input_image> <mask_image> <prompt> <output_image>
  --mask-semantics <white_area_is_edit_area|black_area_is_edit_area|alpha_zero_is_edit_area>
  --format <png|jpg|webp>
```

### 3.4 `upscale_image`

图片超分。映射到 `image.upscale`。

```bash
upscale_image source.png result.png --scale 4
```

参数：

```text
upscale_image <input_image> <output_image>
  --scale <2|4>
  --target-width <px>
  --target-height <px>
  --preserve-faces
  --format <png|jpg|webp>
```

### 3.5 `remove_bg`

图片抠图。映射到 `image.bg_remove`。

```bash
remove_bg product.jpg product_rgba.png
```

参数：

```text
remove_bg <input_image> <output_image>
  --mode <rgba_image|mask>
```

---

## 4. Vision CLI

### 4.1 `ocr_image`

图片 / PDF 页面 OCR。映射到 `vision.ocr`。

```bash
ocr_image page.png result.txt
```

参数：

```text
ocr_image <document> [output_text]
  --level <page|block|line|word>
  --lang <zh,en,...>
  --layout
  --artifact <plain_text|alto_json>
  --json-output <output_json>
```

默认只输出纯文本。`--layout` 或 `--json-output` 会保存结构化 OCR 结果。

### 4.2 `caption_image`

图片描述。映射到 `vision.caption`。

```bash
caption_image image.png
caption_image image.png caption.txt --style short --lang zh-CN
```

参数：

```text
caption_image <image> [output_text]
  --style <short|dense|alt_text>
  --lang <language_tag>
  --n <count>
```

### 4.3 `detect_image`

目标检测。映射到 `vision.detect`。

```bash
detect_image street.png detections.json --classes person,car --threshold 0.3
```

参数：

```text
detect_image <image> <output_json>
  --classes <class1,class2,...>
  --threshold <float>
  --bbox-format <xywh>
  --bbox-unit <px|ratio>
```

### 4.4 `segment_image`

图片分割。映射到 `vision.segment`。

```bash
segment_image image.png masks.json --box 120,80,300,600
```

参数：

```text
segment_image <image> <output_json>
  --box <x,y,width,height>
  --point <x,y>
  --text <prompt>
  --mask-format <rle|polygon>
  --bitmap-dir <dir>
```

---

## 5. Audio CLI

### 5.1 `text_to_speech`

文本转语音。映射到 `audio.tts`。

```bash
text_to_speech "你好，欢迎使用 AICC。" result.mp3
```

参数：

```text
text_to_speech <text> <output_audio>
  --voice-id <id>
  --lang <language_tag>
  --gender <male|female|neutral>
  --style <style>
  --speaker-similarity-required
  --speed <float>
  --format <mp3|wav|ogg>
  --sample-rate <hz>
```

如果传入 `--voice-id --speaker-similarity-required`，CLI 应默认设置 strict route，避免跨 Provider fallback 导致声音不一致。

### 5.2 `speech_to_text`

语音识别。映射到 `audio.asr`。

```bash
speech_to_text meeting.mp3 transcript.txt --lang zh-CN
```

参数：

```text
speech_to_text <input_audio> [output_text]
  --lang <language_tag>
  --timestamps <none|segment|word>
  --diarization
  --format <txt|json|vtt|srt>
  --artifact-dir <dir>
```

`--format txt` 写 transcript。`json`、`vtt`、`srt` 对应 AICC `output_formats`。

### 5.3 `gen_music`

音乐生成。映射到 `audio.music`。

```bash
gen_music "A 30-second cheerful acoustic folk song with guitar." result.mp3
```

参数：

```text
gen_music <prompt> <output_audio>
  --duration <seconds>
  --instrumental
  --lyrics <text_or_file>
  --seed <int>
  --format <mp3|wav|ogg>
```

音乐生成命令默认阻塞到音频产物生成完成。中途停止由调用方直接结束 CLI 进程处理。

### 5.4 `enhance_audio`

音频增强。映射到 `audio.enhance`。

```bash
enhance_audio noisy.wav clean.wav --task denoise
```

参数：

```text
enhance_audio <input_audio> <output_audio>
  --task <denoise|dereverb|separate_voice|normalize>
  --strength <0.0-1.0>
  --return-stems
  --stems-dir <dir>
```

---

## 6. Video CLI

视频类命令默认阻塞到视频产物生成完成。中途停止由调用方直接结束 CLI 进程处理。

### 6.1 `gen_video`

文生视频。映射到 `video.txt2video`。

```bash
gen_video "A cinematic tracking shot through a lantern-lit market at dusk." result.mp4
```

参数：

```text
gen_video <prompt> <output_video>
  --duration <seconds>
  --aspect <16:9|9:16|1:1>
  --resolution <720p|1080p|4k>
  --audio
  --seed <int>
  --fps <int>
  --format <mp4|webm>
```

### 6.2 `img2video`

图生视频。映射到 `video.img2video`。

```bash
img2video start.png "Animate the scene with slow camera movement." result.mp4
```

参数：

```text
img2video <input_image> <prompt> <output_video>
  --duration <seconds>
  --aspect <16:9|9:16|1:1>
  --resolution <720p|1080p|4k>
```

### 6.3 `video2video`

视频风格 / 内容编辑。映射到 `video.video2video`。

```bash
video2video source.mp4 "Shift the color palette to teal and warm backlight." result.mp4
```

参数：

```text
video2video <input_video> <prompt> <output_video>
  --preserve-motion
  --start <seconds>
  --end <seconds>
```

### 6.4 `extend_video`

视频续写。映射到 `video.extend`。

```bash
extend_video previous.mp4 "Continue the shot over the rooftops." result.mp4
```

参数：

```text
extend_video <input_video> <prompt> <output_video>
  --continuation-handle <provider_handle>
  --duration <seconds>
  --resolution <720p|1080p|4k>
```

### 6.5 `upscale_video`

视频超分。映射到 `video.upscale`。

```bash
upscale_video source.mp4 result.mp4 --target-resolution 4k
```

参数：

```text
upscale_video <input_video> <output_video>
  --target-resolution <1080p|4k>
  --denoise
  --sharpen <0.0-1.0>
  --fps <int>
```

---

## 7. Service CLI

### 7.1 `ai_provider`

Provider 查询。

```bash
ai_provider list
ai_provider health
```

映射到：

| command | AICC method |
|---|---|
| `ai_provider list` | `provider.list` |
| `ai_provider health` | `provider.health` |

### 7.2 `ai_quota`

Quota 查询。

```bash
ai_quota
ai_quota --capability image
ai_quota --method image.txt2img
```

映射到 `quota.query`。

---

## 8. 命令到 AICC Method 映射

| CLI | AICC method | capability | 默认 model.alias |
|---|---|---|---|
| `gen_image` | `image.txt2img` | `image` | `image.txt2img` |
| `edit_image` | `image.img2img` | `image` | `image.img2img` |
| `inpaint_image` | `image.inpaint` | `image` | `image.inpaint` |
| `upscale_image` | `image.upscale` | `image` | `image.upscale` |
| `remove_bg` | `image.bg_remove` | `image` | `image.bg_remove` |
| `ocr_image` | `vision.ocr` | `vision` | `vision.ocr` |
| `caption_image` | `vision.caption` | `vision` | `vision.caption` |
| `detect_image` | `vision.detect` | `vision` | `vision.detect` |
| `segment_image` | `vision.segment` | `vision` | `vision.segment` |
| `text_to_speech` | `audio.tts` | `audio` | `audio.tts` |
| `speech_to_text` | `audio.asr` | `audio` | `audio.asr` |
| `gen_music` | `audio.music` | `audio` | `audio.music` |
| `enhance_audio` | `audio.enhance` | `audio` | `audio.enhance` |
| `gen_video` | `video.txt2video` | `video` | `video.txt2video` |
| `img2video` | `video.img2video` | `video` | `video.img2video` |
| `video2video` | `video.video2video` | `video` | `video.video2video` |
| `extend_video` | `video.extend` | `video` | `video.extend` |
| `upscale_video` | `video.upscale` | `video` | `video.upscale` |

---

## 9. Request 构造示例

`gen_image "prompt" result.png --aspect 1:1 --quality high` 构造的 AICC request：

```json
{
  "method": "image.txt2img",
  "params": {
    "capability": "image",
    "model": {
      "alias": "image.txt2img"
    },
    "requirements": {
      "must_features": [],
      "resp_format": "text"
    },
    "payload": {
      "input_json": {
        "prompt": "prompt",
        "n": 1,
        "aspect_ratio": "1:1",
        "quality": "high",
        "output": {
          "media_type": "image/png"
        }
      },
      "resources": [],
      "options": {}
    },
    "policy": {
      "profile": "balanced",
      "allow_fallback": true,
      "runtime_failover": true
    }
  },
  "sys": [1001, "<session_token>", "<trace_id>"]
}
```

同步成功后，CLI 从 `result.artifacts[0].resource` 取生成图片并写入 `result.png`。

---

## 10. 实现建议

### 10.1 安装位置

这些命令应作为 `buckyos-agent tools` 的一部分，打入 OpenDAN / Jarvis 运行环境。建议实现为一个真实 binary 加多个命令别名：

```text
aicc-tool gen_image ...
aicc-tool ocr_image ...

gen_image -> aicc-tool gen_image
ocr_image -> aicc-tool ocr_image
```

这样 Agent 能使用短命令，人类维护时也只有一个入口。

### 10.2 默认连接信息

CLI 默认连接本机 NodeGateway：

```text
http://127.0.0.1:3180/kapi/aicc
```

可通过环境变量覆盖：

```text
AICC_ENDPOINT
BUCKYOS_SESSION_TOKEN
AICC_DEFAULT_PROFILE
AICC_DEFAULT_TIMEOUT
```

### 10.3 JSON 输出契约

所有命令在 `--json` 下应至少包含：

```json
{
  "ok": true,
  "method": "image.txt2img",
  "task_id": "aicc-001",
  "status": "succeeded",
  "outputs": [
    {
      "path": "result.png",
      "mime": "image/png"
    }
  ],
  "response": {}
}
```

失败时：

```json
{
  "ok": false,
  "method": "image.txt2img",
  "error": {
    "code": "route_failed",
    "message": "no provider available"
  }
}
```

### 10.4 MVP 范围

第一批建议只实现：

1. `gen_image`
2. `edit_image`
3. `upscale_image`
4. `remove_bg`
5. `ocr_image`
6. `caption_image`
7. `text_to_speech`
8. `speech_to_text`
9. `gen_video`

这批命令覆盖 Agent 最常用的多模态工具调用场景，同时避开原始 LLM 推理命令。

---

## 11. 非目标

1. 不设计新的 REST AI API。
2. 不在 CLI 中实现 Provider 专有参数透传，除非放入 `--provider-extra <json>`。
3. 不实现 `chat`、`completion`、`ask`、`reason` 等原始 LLM 推理命令。
4. 不在 CLI 内做复杂模型选择逻辑；路由决策属于 AICC。
5. 不为每个 Provider 设计独立命令。
6. 不暴露 embedding、rerank 等主要面向内部检索链路的中间结果命令。
7. 不暴露 `ai_task` 这类任务管理命令；CLI 默认阻塞到产物完成，中途停止直接结束进程。
